// Property tests over generated diffs.
//
// The differential tests next door compare hunkpick with git and need a repository per case;
// these run against the library alone, so they can afford many more cases and cover the parts
// git cannot express — CRLF endings, a missing final newline, a `\ No newline at end of file`
// marker, a preamble before the first entry. What they add over a hand-written fixture is
// shrinking: a failure is reported as the smallest diff that still fails, not as the 30-line
// one that happened to be generated.

use hunkpick::model::{FileContent, Patch};
use hunkpick::{emit, parser, renumber, select, validate};
use proptest::prelude::*;

/// One generated change at a line of the base file. `TwoBlocks` is a single hunk holding two
/// separate change runs with context between them — the shape auto-splitting exists for, and the
/// one the other three never produce.
#[derive(Clone, Debug)]
enum Edit {
    Replace,
    Insert,
    Delete,
    TwoBlocks,
}

/// The shape of a generated diff. Rendering happens in [`render`]; keeping the shape separate
/// is what lets proptest shrink a failure down to "two lines, one deletion, CRLF".
#[derive(Clone, Debug)]
struct DiffShape {
    /// Number of lines in the base file.
    lines: usize,
    /// Edits to apply, each at a line index taken modulo the file length.
    edits: Vec<(usize, Edit)>,
    /// CRLF line endings rather than LF.
    crlf: bool,
    /// The last line of the file has no newline, so the diff carries the marker.
    no_eof_newline: bool,
    /// Mail headers before the first `diff --git`, as `git format-patch` writes them.
    preamble: bool,
    /// The diff itself ends without a final newline (pasted or piped that way).
    no_trailing_newline: bool,
    /// Line numbers sit just below `u32::MAX` — a diff carved out of a huge generated file, and
    /// the range where the overlap arithmetic used to wrap.
    high_line_numbers: bool,
}

fn arb_shape() -> impl Strategy<Value = DiffShape> {
    let edit = prop_oneof![
        Just(Edit::Replace),
        Just(Edit::Insert),
        Just(Edit::Delete),
        Just(Edit::TwoBlocks),
    ];
    (
        2usize..24,
        prop::collection::vec((0usize..24, edit), 1..6),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(
            |(
                lines,
                edits,
                crlf,
                no_eof_newline,
                preamble,
                no_trailing_newline,
                high_line_numbers,
            )| DiffShape {
                lines,
                edits,
                crlf,
                no_eof_newline,
                preamble,
                no_trailing_newline,
                high_line_numbers,
            },
        )
}

/// Render a shape as a unified diff, one hunk per edit with a line of context around it.
///
/// Hand-rolled rather than taken from git: these cases include forms git will not produce on
/// demand (a mail preamble around an arbitrary hunk, a missing final newline), and the point is
/// to feed the parser byte sequences, not to check what git writes.
fn render(shape: &DiffShape) -> Vec<u8> {
    let nl: &[u8] = if shape.crlf { b"\r\n" } else { b"\n" };
    let mut out: Vec<u8> = Vec::new();
    if shape.preamble {
        out.extend_from_slice(b"From: Someone <someone@example.invalid>");
        out.extend_from_slice(nl);
        out.extend_from_slice(b"Subject: [PATCH] generated");
        out.extend_from_slice(nl);
        out.extend_from_slice(nl);
    }
    out.extend_from_slice(b"diff --git a/f b/f");
    out.extend_from_slice(nl);
    out.extend_from_slice(b"--- a/f");
    out.extend_from_slice(nl);
    out.extend_from_slice(b"+++ b/f");
    out.extend_from_slice(nl);

    // A diff of a file whose lines are numbered near the top of the u32 range. The bodies are
    // unaffected; only the header anchors move, which is where the overlap and anchor arithmetic
    // used to wrap (a debug build panicked, a release build compared a meaningless value).
    let base = if shape.high_line_numbers {
        u32::MAX as usize - 10_000
    } else {
        0
    };

    // Hunks are emitted in ascending, non-overlapping old-side order: the parser rejects an
    // input whose hunks overlap, and a rejected input would test nothing.
    let mut at = 1usize;
    let mut delta: i64 = 0;
    let mut hunks = 0usize;
    for (raw, edit) in &shape.edits {
        if at + 2 > shape.lines {
            break;
        }
        let start = at + raw % (shape.lines - at + 1).max(1);
        let start = start.min(shape.lines.saturating_sub(1)).max(at);
        let (old_lines, new_lines, body): (usize, usize, Vec<(u8, String)>) = match edit {
            Edit::Replace => (
                2,
                2,
                vec![
                    (b' ', format!("line {start}")),
                    (b'-', format!("line {}", start + 1)),
                    (b'+', format!("changed {}", start + 1)),
                ],
            ),
            Edit::Insert => (
                1,
                2,
                vec![
                    (b' ', format!("line {start}")),
                    (b'+', format!("inserted at {start}")),
                ],
            ),
            Edit::Delete => (
                2,
                1,
                vec![
                    (b' ', format!("line {start}")),
                    (b'-', format!("line {}", start + 1)),
                ],
            ),
            // Two change runs separated by a context line: one hunk, two sub-hunks. This is what
            // auto-splitting is for, and none of the shapes above reach it.
            Edit::TwoBlocks => (
                5,
                5,
                vec![
                    (b' ', format!("line {start}")),
                    (b'-', format!("line {}", start + 1)),
                    (b'+', format!("changed {}", start + 1)),
                    (b' ', format!("line {}", start + 2)),
                    (b'-', format!("line {}", start + 3)),
                    (b'+', format!("changed {}", start + 3)),
                    (b' ', format!("line {}", start + 4)),
                ],
            ),
        };
        let new_start = (start as i64 + delta).max(1) as usize;
        delta += new_lines as i64 - old_lines as i64;
        out.extend_from_slice(
            format!(
                "@@ -{} +{} @@",
                range(start + base, old_lines),
                range(new_start + base, new_lines)
            )
            .as_bytes(),
        );
        out.extend_from_slice(nl);
        for (marker, text) in &body {
            out.push(*marker);
            out.extend_from_slice(text.as_bytes());
            out.extend_from_slice(nl);
        }
        at = start + old_lines + 1;
        hunks += 1;
    }

    // The marker only ever qualifies the very last line of the whole diff — anywhere else it
    // would describe a file that ends in the middle of a hunk — so it is appended once the
    // hunks are rendered. Deciding it inside the loop against `shape.edits.len()` dropped it
    // silently whenever the loop stopped early on the `at + 2 > shape.lines` guard, and the
    // shape's `no_eof_newline` then went untested rather than tested.
    if shape.no_eof_newline && hunks > 0 {
        out.extend_from_slice(b"\\ No newline at end of file");
        out.extend_from_slice(nl);
    }

    if shape.no_trailing_newline {
        while out.last() == Some(&b'\n') || out.last() == Some(&b'\r') {
            out.pop();
        }
    }
    out
}

/// One side of a hunk header the way git spells it: the `,1` suffix of a single-line range is
/// omitted. The round-trip promise is about git-canonical input, so the generator has to write
/// the canonical form — writing `-1,1` instead was the first thing these tests caught.
fn range(start: usize, count: usize) -> String {
    if count == 1 {
        start.to_string()
    } else {
        format!("{start},{count}")
    }
}

/// Sub-hunk indices of the single file of a parsed patch.
fn sub_hunk_count(patch: &Patch) -> usize {
    select::build_view(patch)
        .first()
        .map(|subs| subs.len())
        .unwrap_or(0)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Whatever comes in comes back out: the parser keeps every byte it does not understand,
    /// and the renderer puts it where it was. This is the promise `emit` documents, and the
    /// cheapest way to notice a detail being normalised away.
    #[test]
    fn parse_then_emit_is_the_input(shape in arb_shape()) {
        let src = render(&shape);
        let patch = parser::parse(&src).expect("generated diffs are well formed");
        prop_assert_eq!(emit::emit(&patch), src);
    }

    /// A selection either fails as a usage error or yields a diff that passes the internal
    /// check. It must never produce a diff the tool itself would reject — that combination is
    /// exit code 70 on valid input, which is what the zero-count defect looked like.
    #[test]
    fn a_selection_is_never_internally_inconsistent(shape in arb_shape(), mask in any::<u32>()) {
        let src = render(&shape);
        let patch = parser::parse(&src).expect("generated diffs are well formed");
        let count = sub_hunk_count(&patch);
        prop_assume!(count > 0);

        let picks: Vec<String> = (1..=count)
            .filter(|i| mask & (1 << (i % 32)) != 0)
            .map(|i| i.to_string())
            .collect();
        prop_assume!(!picks.is_empty());

        let selectors = select::parse_selectors(&picks).expect("indices always parse");
        let result = select::select(&patch, &selectors).expect("in-range indices resolve");
        prop_assert_eq!(validate::validate_internal(&result), Ok(()));
        // And the result is acceptable as input in turn, so a pipeline can be chained.
        prop_assert_eq!(validate::validate_input(&result), Ok(()));
    }

    /// Recomputing the new-side anchors of a diff that already has correct ones changes
    /// nothing: the pass has to be idempotent, or repeated `select` invocations would drift.
    #[test]
    fn renumbering_is_idempotent(shape in arb_shape()) {
        let src = render(&shape);
        let mut once = parser::parse(&src).expect("generated diffs are well formed");
        renumber::renumber_new_side(&mut once);
        let mut twice = once.clone();
        renumber::renumber_new_side(&mut twice);
        prop_assert_eq!(twice, once);
    }

    /// Selecting every sub-hunk keeps every change of the input: the same added and deleted
    /// line counts come out. Auto-splitting may redraw the hunk boundaries, but it may not
    /// lose or invent a line.
    #[test]
    fn selecting_everything_preserves_the_change_counts(shape in arb_shape()) {
        let src = render(&shape);
        let patch = parser::parse(&src).expect("generated diffs are well formed");
        prop_assume!(sub_hunk_count(&patch) > 0);

        let selectors = select::parse_selectors(&["*".to_string()]).unwrap();
        let result = select::select(&patch, &selectors).expect("'*' always resolves");
        prop_assert_eq!(change_counts(&result), change_counts(&patch));
    }
}

/// Total (added, deleted) line counts over every text hunk of a patch.
fn change_counts(patch: &Patch) -> (u32, u32) {
    let mut added = 0;
    let mut deleted = 0;
    for f in &patch.files {
        if let FileContent::Text(hunks) = &f.content {
            for h in hunks {
                let (a, d) = h.change_counts();
                added += a;
                deleted += d;
            }
        }
    }
    (added, deleted)
}
