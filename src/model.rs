#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Add,
    Del,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    /// Content without the leading +/-/space marker and without the trailing '\n'.
    /// A trailing '\r' (CRLF input) is preserved here. Stored as raw bytes so any input
    /// encoding (or invalid UTF-8) round-trips unchanged.
    pub text: Vec<u8>,
    /// True when this line is followed by "\ No newline at end of file".
    pub no_newline: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    /// Text after the second `@@` on the hunk header (without leading space). May be empty.
    pub section: Vec<u8>,
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileContent {
    Text(Vec<Hunk>),
    /// Binary patch body lines, stored verbatim (without trailing '\n').
    Binary(Vec<Vec<u8>>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiff {
    /// Raw header lines before the first hunk, verbatim, without trailing '\n'.
    pub headers: Vec<Vec<u8>>,
    pub old_path: Option<Vec<u8>>,
    pub new_path: Option<Vec<u8>>,
    pub content: FileContent,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Patch {
    pub files: Vec<FileDiff>,
}

impl FileDiff {
    /// Best-effort display path: new path, else old path, decoded lossily. Empty if neither.
    /// For display and error messages only; the emitted diff keeps the original path bytes.
    pub fn display_path(&self) -> String {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default()
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
                    no_newline: false,
                },
                Line {
                    kind: LineKind::Del,
                    text: b"b".to_vec(),
                    no_newline: false,
                },
                Line {
                    kind: LineKind::Add,
                    text: b"c".to_vec(),
                    no_newline: false,
                },
            ],
        };
        assert_eq!(h.change_counts(), (1, 1));
    }
}
