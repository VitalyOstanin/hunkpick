use crate::model::*;

// FNV-1a 64-bit. Chosen for a fixed, portable, dependency-free hash: the id must be identical
// across runs and platforms (it is shared between a `list` invocation and a later `select`),
// which `std`'s `DefaultHasher` does not guarantee. Cryptographic strength is not needed —
// accidental collisions between distinct sub-hunks are caught by a content comparison at
// `select` time; the hash only has to spread unrelated inputs well.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

struct Fnv1a(u64);

impl Fnv1a {
    fn new() -> Self {
        Fnv1a(FNV_OFFSET)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }

    fn write_u8(&mut self, b: u8) {
        self.write(&[b]);
    }
}

/// Feed the identity-defining bytes of `(file, sub)` into `h`: the file paths and, for each
/// *changed* (added or deleted) line, its kind marker, raw text, and no-newline flag. Context
/// lines, the `@@` line numbers, and the section header are intentionally excluded, so the id
/// depends only on what the sub-hunk changes and in which file — not on its surrounding
/// context. A NUL field separator after each variable-length field keeps adjacent fields from
/// running together (e.g. path "ab"+"" vs "a"+"b").
fn feed(h: &mut Fnv1a, file: &FileDiff, sub: &Hunk) {
    h.write(file.new_path.as_deref().unwrap_or_default());
    h.write_u8(0);
    h.write(file.old_path.as_deref().unwrap_or_default());
    h.write_u8(0);
    for (_, l) in sub.changed_lines() {
        let marker = match l.kind {
            LineKind::Add => b'+',
            LineKind::Del => b'-',
            LineKind::Context => unreachable!("changed_lines excludes context lines"),
        };
        h.write_u8(marker);
        h.write(&l.text);
        h.write_u8(0);
        h.write_u8(l.no_newline as u8);
    }
}

/// 64-bit content hash of a sub-hunk; the raw value backing [`subhunk_id`].
pub fn subhunk_hash(file: &FileDiff, sub: &Hunk) -> u64 {
    let mut h = Fnv1a::new();
    feed(&mut h, file, sub);
    h.0
}

/// Stable content hash identifying a sub-hunk, as 16 lowercase hex digits. The id is a function
/// of the file's paths and the sub-hunk's *changed* lines (kind + bytes + no-newline flag)
/// only; it deliberately excludes context lines, the `@@` line numbers, and the section header,
/// so the same change keeps the same id across a re-diff even when its line numbers shift or its
/// surrounding context changes (e.g. after a neighbouring sub-hunk is staged and the hunk is
/// re-split). Byte-identical changes therefore share an id; see [`subhunk_hash`].
pub fn subhunk_id(file: &FileDiff, sub: &Hunk) -> String {
    format!("{:016x}", subhunk_hash(file, sub))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: LineKind, text: &str) -> Line {
        Line {
            kind,
            text: text.as_bytes().to_vec(),
            no_newline: false,
        }
    }

    fn file(new_path: &str) -> FileDiff {
        FileDiff {
            headers: Vec::new(),
            old_path: Some(new_path.as_bytes().to_vec()),
            new_path: Some(new_path.as_bytes().to_vec()),
            content: FileContent::Text(Vec::new()),
        }
    }

    fn hunk(old_start: u32, new_start: u32, lines: Vec<Line>) -> Hunk {
        Hunk {
            old_start,
            old_lines: 0,
            new_start,
            new_lines: 0,
            section: Vec::new(),
            lines,
        }
    }

    fn sample_lines() -> Vec<Line> {
        vec![
            line(LineKind::Context, "a"),
            line(LineKind::Del, "b"),
            line(LineKind::Add, "B"),
            line(LineKind::Context, "c"),
        ]
    }

    #[test]
    fn id_is_16_lowercase_hex() {
        let f = file("src/a.rs");
        let h = hunk(1, 1, sample_lines());
        let id = subhunk_id(&f, &h);
        assert_eq!(id.len(), 16, "id must be 16 hex chars, got {id:?}");
        assert!(
            id.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "id must be lowercase hex: {id:?}"
        );
    }

    #[test]
    fn id_is_stable_across_line_number_shift() {
        let f = file("src/a.rs");
        // Same lines and section, different @@ line numbers.
        let h1 = hunk(10, 10, sample_lines());
        let h2 = hunk(48, 49, sample_lines());
        assert_eq!(
            subhunk_id(&f, &h1),
            subhunk_id(&f, &h2),
            "id must not depend on @@ line numbers"
        );
    }

    #[test]
    fn id_is_stable_across_section_change() {
        let f = file("src/a.rs");
        let mut h1 = hunk(10, 10, sample_lines());
        let mut h2 = hunk(10, 10, sample_lines());
        h1.section = b"fn foo()".to_vec();
        h2.section = b"fn bar()".to_vec();
        assert_eq!(
            subhunk_id(&f, &h1),
            subhunk_id(&f, &h2),
            "id must not depend on the @@ section header"
        );
    }

    #[test]
    fn id_differs_for_different_content() {
        let f = file("src/a.rs");
        let h1 = hunk(1, 1, sample_lines());
        let mut other = sample_lines();
        other[2] = line(LineKind::Add, "X");
        let h2 = hunk(1, 1, other);
        assert_ne!(subhunk_id(&f, &h1), subhunk_id(&f, &h2));
    }

    #[test]
    fn id_unchanged_when_only_context_differs() {
        // Context lines are NOT part of the id — only the changed (+/-) lines and the paths
        // are. The same edit with different surrounding context (e.g. after a neighbouring
        // change is staged and the hunk is re-split with full context) keeps the same id.
        // This pins the stability boundary: the id is stable across @@ line-number shifts AND
        // across changes to its own context lines; it changes only when its +/- lines change.
        let f = file("src/a.rs");
        let base = sample_lines();
        let mut with_extra_ctx = sample_lines();
        with_extra_ctx.insert(0, line(LineKind::Context, "z"));
        assert_eq!(
            subhunk_id(&f, &hunk(1, 1, base)),
            subhunk_id(&f, &hunk(1, 1, with_extra_ctx))
        );
    }

    #[test]
    fn id_differs_for_different_path() {
        let h = hunk(1, 1, sample_lines());
        assert_ne!(
            subhunk_id(&file("src/a.rs"), &h),
            subhunk_id(&file("src/b.rs"), &h)
        );
    }

    #[test]
    fn id_differs_when_line_kind_differs() {
        // Same text, different changed-line kind (a deletion vs an addition) must not collide:
        // removing "x" is a different change from adding "x".
        let f = file("src/a.rs");
        let h1 = hunk(1, 1, vec![line(LineKind::Del, "x")]);
        let h2 = hunk(1, 1, vec![line(LineKind::Add, "x")]);
        assert_ne!(subhunk_id(&f, &h1), subhunk_id(&f, &h2));
    }

    #[test]
    fn id_differs_on_no_newline_flag() {
        let f = file("src/a.rs");
        let h1 = hunk(1, 1, vec![line(LineKind::Add, "x")]);
        let mut nn = line(LineKind::Add, "x");
        nn.no_newline = true;
        let h2 = hunk(1, 1, vec![nn]);
        assert_ne!(subhunk_id(&f, &h1), subhunk_id(&f, &h2));
    }
}
