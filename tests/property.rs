// Property tests over generated diffs.
//
// The differential tests next door compare hunkpick with git and need a repository per case;
// these run against the library alone, so they can afford many more cases and cover the parts
// git cannot express — CRLF endings, a missing final newline, a `\ No newline at end of file`
// marker, a preamble before the first entry. What they add over a hand-written fixture is
// shrinking: a failure is reported as the smallest diff that still fails, not as the 30-line
// one that happened to be generated.

use hunkpick::model::{FileContent, LineKind, Patch};
use hunkpick::{emit, list, parser, renumber, select, split, validate};
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
    /// A replacement followed by two context lines, so the hunk has a context line strictly
    /// inside its trailing run. Cutting there leaves a last piece with no changes at all, which
    /// `split` drops — the only generated shape where a cut moves where the diff ends.
    TrailingContext,
    /// A replacement whose changed lines carry a fixed text rather than one derived from the
    /// line number. A content id hashes the path and the changed lines only, so two of these in
    /// one diff share an id — the shape `@id` collision handling exists for, and the one every
    /// other edit here avoids by construction.
    Repeated,
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
    /// The `-- ` / version signature `git format-patch` appends after the last hunk. Held in
    /// `FileDiff::trailer` rather than among the hunks, and moved by `split_file_hunk`: a line
    /// left at its old position ends up emitted between two pieces, where `git apply` reads it
    /// as a fragment without a header.
    signature: bool,
}

fn arb_shape() -> impl Strategy<Value = DiffShape> {
    let edit = prop_oneof![
        Just(Edit::Replace),
        Just(Edit::Insert),
        Just(Edit::Delete),
        Just(Edit::TwoBlocks),
        Just(Edit::Repeated),
        Just(Edit::TrailingContext),
    ];
    (
        2usize..24,
        prop::collection::vec((0usize..24, edit), 1..6),
        any::<bool>(),
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
                signature,
            )| DiffShape {
                lines,
                edits,
                crlf,
                no_eof_newline,
                preamble,
                no_trailing_newline,
                high_line_numbers,
                signature,
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
            // Two trailing context lines after the change: the first of them is a context line
            // strictly inside the hunk, so `--at` can cut there and leave a piece with nothing
            // in it but context.
            Edit::TrailingContext => (
                4,
                4,
                vec![
                    (b' ', format!("line {start}")),
                    (b'-', format!("line {}", start + 1)),
                    (b'+', format!("changed {}", start + 1)),
                    (b' ', format!("line {}", start + 2)),
                    (b' ', format!("line {}", start + 3)),
                ],
            ),
            // Changed lines that do not mention `start`, so two of these hash to the same
            // content id however far apart they sit. The context still moves with the hunk.
            Edit::Repeated => (
                2,
                2,
                vec![
                    (b' ', format!("line {start}")),
                    (b'-', "repeated".to_string()),
                    (b'+', "REPEATED".to_string()),
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

    // After the marker, as `git format-patch` writes it: the trailing space on `-- ` is part of
    // the signature and is what tells a mail reader where the patch ends.
    if shape.signature && hunks > 0 {
        out.extend_from_slice(b"-- ");
        out.extend_from_slice(nl);
        out.extend_from_slice(b"2.53.0");
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

    /// A content id addresses every sub-hunk whose changed lines are byte-identical, and
    /// `list --json` reports how many that is as `id_count`. The two have to agree: the
    /// documented batch flow reads an id out of the listing and passes it back, and a selection
    /// that took fewer sub-hunks than the listing promised would stage part of a change without
    /// saying so. Reachable only because one generated edit repeats its text — every other
    /// shape derives the changed lines from the line number, so no two ever collide.
    #[test]
    fn an_id_selects_exactly_the_sub_hunks_the_listing_counts(shape in arb_shape()) {
        let src = render(&shape);
        let patch = parser::parse(&src).expect("generated diffs are well formed");
        prop_assume!(sub_hunk_count(&patch) > 0);

        let listing: serde_json::Value =
            serde_json::from_str(&list::list_json(&patch)).expect("the listing is JSON");
        let hunks = listing[0]["hunks"].as_array().expect("a file with sub-hunks");
        // The id shared by the most sub-hunks: with a repeated edit it names several, otherwise
        // it names one, and both are worth asserting.
        let (id, id_count) = hunks
            .iter()
            .map(|h| {
                (
                    h["id"].as_str().expect("an id").to_string(),
                    h["id_count"].as_u64().expect("a count") as usize,
                )
            })
            .max_by_key(|(_, count)| *count)
            .expect("at least one sub-hunk");

        let selectors = select::parse_selectors(&[format!("@{id}")]).expect("an id parses");
        let result = select::select(&patch, &selectors).expect("an id from the listing resolves");
        prop_assert_eq!(sub_hunk_count(&result), id_count);
        prop_assert_eq!(validate::validate_internal(&result), Ok(()));
    }

    /// Splitting a hunk moves every later hunk down, and the lines recorded after a hunk have to
    /// move with it. A signature left at its old position is emitted between two pieces, where
    /// `git apply` reads it as a patch fragment without a header — and hunkpick, which checks
    /// hunk bodies rather than the lines between them, reports success. Nothing generated
    /// reached this before: `git diff` output carries no trailing lines at all.
    #[test]
    fn splitting_keeps_the_signature_after_the_last_piece(shape in arb_signed_shape()) {
        let src = render(&shape);
        let patch = parser::parse(&src).expect("generated diffs are well formed");
        prop_assume!(!patch.files[0].trailer.is_empty());
        let FileContent::Text(hunks) = &patch.files[0].content else {
            unreachable!("the generator writes text files")
        };
        // Only a hunk with a context line strictly inside it can be cut; that is the two-block
        // shape, which `arb_signed_shape` puts first but the render loop may still drop.
        let Some((hi, cut)) = hunks
            .iter()
            .enumerate()
            .find_map(|(i, h)| interior_context_line(h).map(|c| (i, c)))
        else {
            return Ok(());
        };

        let mut out = patch.clone();
        let pieces = split::split_file_hunk(&mut out.files[0], hi, &[cut])
            .expect("a cut at an interior context line");
        prop_assume!(pieces > 1);

        let hunk_count = match &out.files[0].content {
            FileContent::Text(h) => h.len(),
            FileContent::Binary(_) => unreachable!(),
        };
        for (at, line) in &out.files[0].trailer {
            prop_assert_eq!(
                *at,
                hunk_count,
                "the signature {:?} has to stay after the last piece",
                String::from_utf8_lossy(line)
            );
        }
        // And the result reads back as what it is, rather than as a fragment without a header.
        let text = emit::emit(&out);
        let again = parser::parse(&text).expect("a split diff parses");
        prop_assert_eq!(&again, &out);
    }

    /// A diff that arrived without its final newline may be re-emitted that way only while the
    /// result still ends on the line the input ended on. Cutting a hunk can drop the piece that
    /// carried the tail, and then removing the newline truncates a line the caller never
    /// addressed — `git apply` calls the result a corrupt patch, and hunkpick's own check,
    /// which does not look past the last hunk, does not.
    #[test]
    fn splitting_drops_the_final_newline_only_where_the_input_ended(shape in arb_signed_shape()) {
        let mut shape = shape;
        shape.signature = false;
        shape.no_trailing_newline = true;
        let src = render(&shape);
        prop_assume!(!src.ends_with(b"\n"));
        let patch = parser::parse(&src).expect("generated diffs are well formed");
        let FileContent::Text(hunks) = &patch.files[0].content else {
            unreachable!("the generator writes text files")
        };
        // The last splittable hunk, not the first: only a cut of the last hunk can move where
        // the diff ends, which is the whole point of the rule under test.
        let Some((hi, cut)) = hunks
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, h)| last_interior_context_line(h).map(|c| (i, c)))
        else {
            return Ok(());
        };

        let mut out = patch.clone();
        split::split_patch_hunk(&mut out, 0, hi, &[cut])
            .expect("a cut at an interior context line");
        let text = emit::emit(&out);
        prop_assume!(!text.is_empty());
        if !text.ends_with(b"\n") {
            // The output ends mid-line, so that line has to be the one the input ended on.
            let last_line = match text.iter().rposition(|&b| b == b'\n') {
                Some(i) => &text[i + 1..],
                None => &text[..],
            };
            prop_assert!(
                src.ends_with(last_line),
                "the output ends on {:?}, which is not where the input ended",
                String::from_utf8_lossy(last_line)
            );
        }
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

/// The last new-file line number of a context line strictly inside the hunk. Cutting there
/// leaves the smallest possible tail piece, which is what drops when it holds no change.
fn last_interior_context_line(h: &hunkpick::model::Hunk) -> Option<u32> {
    let mut new_no = h.new_start;
    let last = h.lines.len() - 1;
    let mut found = None;
    for (i, l) in h.lines.iter().enumerate() {
        match l.kind {
            LineKind::Del => continue,
            LineKind::Context | LineKind::Add => {
                let here = new_no;
                new_no += 1;
                if i > 0 && i < last && matches!(l.kind, LineKind::Context) {
                    found = Some(here);
                }
            }
        }
    }
    found
}

/// A shape that carries the signature and, in front of its own edits, a two-block hunk — the
/// only generated shape with a context line strictly inside it, and therefore the only one
/// `split --at` can cut. Without it nearly every case is rejected before it tests anything.
fn arb_signed_shape() -> impl Strategy<Value = DiffShape> {
    arb_shape().prop_map(|mut shape| {
        shape.signature = true;
        shape.edits.insert(0, (0, Edit::TwoBlocks));
        shape
    })
}

/// A new-file line number of a context line strictly inside the hunk — a place `split --at`
/// accepts. `None` when the hunk has no interior context line, which is most of them.
fn interior_context_line(h: &hunkpick::model::Hunk) -> Option<u32> {
    let mut new_no = h.new_start;
    let last = h.lines.len() - 1;
    let mut found = None;
    for (i, l) in h.lines.iter().enumerate() {
        match l.kind {
            LineKind::Del => continue,
            LineKind::Context | LineKind::Add => {
                let here = new_no;
                new_no += 1;
                if found.is_none() && i > 0 && i < last && matches!(l.kind, LineKind::Context) {
                    found = Some(here);
                }
            }
        }
    }
    found
}
