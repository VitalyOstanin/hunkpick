/// What one body line of a hunk does to the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// Unchanged line, present on both sides (` ` marker).
    Context,
    /// Line the diff adds (`+` marker).
    Add,
    /// Line the diff removes (`-` marker).
    Del,
}

/// One body line of a hunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// Whether the line is context, an addition or a deletion.
    pub kind: LineKind,
    /// Content without the leading +/-/space marker and without the trailing '\n'.
    /// A trailing '\r' (CRLF input) is preserved here. Stored as raw bytes so any input
    /// encoding (or invalid UTF-8) round-trips unchanged.
    pub text: Vec<u8>,
    /// The `\ No newline at end of file` marker that follows this line, as it arrived and
    /// without its line ending — `Some(b"\\ No newline at end of file")`, or with a trailing
    /// `\r` in a CRLF diff. `None` when the line ends normally. Kept verbatim rather than as a
    /// flag so the marker is emitted with the line ending it came with.
    pub no_newline: Option<Vec<u8>>,
}

/// One hunk: the `@@ -old_start,old_lines +new_start,new_lines @@ section` header and its body.
/// After auto-splitting, a "hunk" is a sub-hunk — the unit hunkpick addresses and emits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// First line of the hunk on the old side, 1-based.
    pub old_start: u32,
    /// How many old-side lines the hunk covers (context + deletions).
    pub old_lines: u32,
    /// First line of the hunk on the new side, 1-based. Not independent: it follows from the
    /// old side plus what the hunks before it in the same file change (see [`crate::renumber`]).
    pub new_start: u32,
    /// How many new-side lines the hunk covers (context + additions).
    pub new_lines: u32,
    /// Text after the second `@@` on the hunk header (without leading space). May be empty.
    pub section: Vec<u8>,
    /// Body lines in file order.
    pub lines: Vec<Line>,
}

/// The body of one file entry: hunks for a text file, raw lines for a binary one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileContent {
    /// Hunks of a text file, in file order. May be empty for a header-only entry (a pure
    /// rename or a mode change).
    Text(Vec<Hunk>),
    /// Binary patch body lines, stored verbatim (without trailing '\n').
    Binary(Vec<Vec<u8>>),
}

/// One file entry of a diff: its header lines, its paths and its body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiff {
    /// Raw header lines before the first hunk, verbatim, without trailing '\n'.
    pub headers: Vec<Vec<u8>>,
    /// Raw lines that follow a hunk body rather than precede the first hunk: a blank separator
    /// between hunks, the `-- \n<version>` signature `git format-patch` appends, or trailing
    /// junk. Each is paired with the number of hunks seen before it, so emitting restores its
    /// place. Kept apart from `headers` because emitting such a line up front moves it above
    /// the first `@@` and `git apply` then rejects the whole diff as garbage.
    ///
    /// Ordered by that position, non-decreasing: the parser appends entries as it reads the
    /// file, and [`crate::split::split_file_hunk`] shifts them monotonically. Emitting relies on
    /// the order to walk the list once instead of rescanning it for every hunk, so a caller
    /// building a `FileDiff` by hand keeps the entries in it.
    pub trailer: Vec<(usize, Vec<u8>)>,
    /// Old-side path with the `a/` prefix stripped and git quoting decoded; `None` until a
    /// `--- ` or `diff --git` line supplies it. Raw bytes, so a non-UTF-8 name round-trips.
    pub old_path: Option<Vec<u8>>,
    /// New-side path, in the same form as [`FileDiff::old_path`].
    pub new_path: Option<Vec<u8>>,
    /// The file's body.
    pub content: FileContent,
}

/// A parsed unified diff: its file entries in input order, plus whatever preceded the first
/// one.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Patch {
    /// Raw lines before the first file entry, verbatim and without trailing '\n': the mail
    /// headers, commit message and diffstat of a `git format-patch` output, or any other
    /// preamble. Kept so the diff renders back unchanged — the counterpart of
    /// [`FileDiff::trailer`], which holds the mail's footer.
    pub preamble: Vec<Vec<u8>>,
    /// File entries in the order they appear in the diff.
    pub files: Vec<FileDiff>,
    /// True when the input's last line had no line ending — a diff pasted or piped without its
    /// final newline. Spelled negatively so the common case is the `Default` value, matching
    /// [`Line::no_newline`].
    pub no_trailing_newline: bool,
}

impl FileDiff {
    /// Best-effort display path: new path, else old path, decoded lossily. Empty if neither.
    /// `/dev/null` is skipped: it is the placeholder git writes for the missing side of a
    /// creation or deletion, so the real name lives on the other side.
    /// For display and error messages only; the emitted diff keeps the original path bytes.
    pub fn display_path(&self) -> String {
        let real = |p: &Option<Vec<u8>>| match p.as_deref() {
            Some(b"/dev/null") | None => None,
            Some(b) => Some(b.to_vec()),
        };
        real(&self.new_path)
            .or_else(|| real(&self.old_path))
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    }

    /// How many hunks the entry carries; see [`FileContent::hunk_count`].
    pub fn hunk_count(&self) -> usize {
        self.content.hunk_count()
    }
}

impl FileContent {
    /// How many hunks the body carries. A binary body has none: its payload is not split into
    /// hunks, and every caller that counts them means "positions a trailer line can follow" or
    /// "sub-hunks to address", which is zero for it.
    pub fn hunk_count(&self) -> usize {
        match self {
            FileContent::Text(hunks) => hunks.len(),
            FileContent::Binary(_) => 0,
        }
    }
}

/// (context, added, deleted) line counts over a slice of lines. Shared by the few places
/// that need per-kind tallies (`change_counts`, header recomputation in `split`, the
/// internal consistency check in `validate`) so the count logic lives in one spot.
pub(crate) fn count_kinds(lines: &[Line]) -> (u32, u32, u32) {
    let mut ctx = 0;
    let mut add = 0;
    let mut del = 0;
    for l in lines {
        match l.kind {
            LineKind::Context => ctx += 1,
            LineKind::Add => add += 1,
            LineKind::Del => del += 1,
        }
    }
    (ctx, add, del)
}

impl Hunk {
    /// (added, deleted) line counts.
    pub fn change_counts(&self) -> (u32, u32) {
        let (_, add, del) = count_kinds(&self.lines);
        (add, del)
    }

    /// The sub-hunk's changed (`+`/`-`) lines in body order, each paired with its 1-based index
    /// over `1..=changed` (additions and deletions share one numbering). Context lines are
    /// excluded. This is the single source of truth for "changed lines of a sub-hunk": the
    /// content id ([`crate::subhunk_id`]), `list --json`'s `changed_lines`, and the
    /// `INDEX@L<set>` slice numbering are all built from it, so their numbering agrees by
    /// construction.
    pub fn changed_lines(&self) -> impl Iterator<Item = (usize, &Line)> {
        self.lines
            .iter()
            .filter(|l| !matches!(l.kind, LineKind::Context))
            .enumerate()
            .map(|(idx, l)| (idx + 1, l))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_path_of_a_deletion_is_the_old_path() {
        // A deleted file has `+++ /dev/null`; showing that as the file's name makes every
        // deletion in a listing look alike and is useless as a selector.
        let f = FileDiff {
            headers: Vec::new(),
            trailer: Vec::new(),
            old_path: Some(b"f2".to_vec()),
            new_path: Some(b"/dev/null".to_vec()),
            content: FileContent::Text(Vec::new()),
        };
        assert_eq!(f.display_path(), "f2");
    }

    #[test]
    fn change_counts_counts_add_and_del() {
        let h = Hunk {
            old_start: 1,
            old_lines: 2,
            new_start: 1,
            new_lines: 2,
            section: Vec::new(),
            lines: vec![
                Line {
                    kind: LineKind::Context,
                    text: b"a".to_vec(),
                    no_newline: None,
                },
                Line {
                    kind: LineKind::Del,
                    text: b"b".to_vec(),
                    no_newline: None,
                },
                Line {
                    kind: LineKind::Add,
                    text: b"c".to_vec(),
                    no_newline: None,
                },
            ],
        };
        assert_eq!(h.change_counts(), (1, 1));
    }
}
