use crate::model::*;
use std::fmt;

/// Why an input diff could not be parsed. Every variant is a usage error (exit code 2).
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    /// A `@@` line does not match `@@ -os,ol +ns,nl @@`. Carries the line.
    BadHunkHeader(String),
    /// Diff content that cannot occur in a well-formed diff (e.g. a hunk inside a binary
    /// file entry). Carries a description.
    Unexpected(String),
    /// The input is a combined diff — the n-way format git writes for a merge (`diff --cc`,
    /// `@@@` headers). Its body carries one marker column per parent, so it is not a two-sided
    /// unified diff and cannot be addressed or sliced. Carries the line that revealed it.
    Combined(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::BadHunkHeader(s) => write!(f, "malformed hunk header: {s}"),
            ParseError::Unexpected(s) => write!(f, "unexpected diff content: {s}"),
            ParseError::Combined(s) => write!(
                f,
                "combined (merge) diff is not supported, hunkpick reads two-sided \
                 unified diffs: {s}"
            ),
        }
    }
}

/// Lets callers treat it as a boxed [`std::error::Error`], as the Rust API guidelines ask
/// of a public error type.
impl std::error::Error for ParseError {}

/// How much of the hunk declared by the last `@@` header is still unread: the counts from
/// `-os,ol +ns,nl` minus what the body has already consumed. Zero on both sides means the hunk
/// is complete, so a following line belongs to whatever comes after it.
#[derive(Default)]
struct Remaining {
    old: u32,
    new: u32,
}

impl Remaining {
    fn exhausted(&self) -> bool {
        self.old == 0 && self.new == 0
    }
}

/// What reading one body line did.
enum BodyLine {
    /// The line was consumed into the hunk body.
    Consumed,
    /// The line does not belong to the body (a non-body line, or a body line past the declared
    /// count): the hunk has ended and the line is to be reinterpreted as a header.
    HunkEnded,
}

/// Per-file parse state that has to be reset whenever a new file starts.
#[derive(Default)]
struct FileState {
    /// Inside a hunk body whose declared lines are not yet exhausted.
    in_hunk: bool,
    /// The file has already had at least one hunk (needed to detect the next file in a plain
    /// diff).
    saw_hunk: bool,
    /// The file has had its `+++ ` line, so it is fully declared: in a plain diff the next
    /// `--- ` then opens the next file even if this one has no hunks.
    saw_marker_pair: bool,
    remaining: Remaining,
}

/// Parse a unified diff — git-generated or plain — into a [`Patch`], preserving every byte
/// needed to render it back unchanged: header lines, trailing lines after a hunk, `\r` of CRLF
/// input, and paths as raw bytes.
pub fn parse(input: &[u8]) -> Result<Patch, ParseError> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut preamble: Vec<Vec<u8>> = Vec::new();
    let mut cur: Option<FileDiff> = None;
    let mut st = FileState::default();

    let mut lines = input.split(|&b| b == b'\n').peekable();
    while let Some(line) = lines.next() {
        let is_last_empty = line.is_empty() && lines.peek().is_none();
        if is_last_empty {
            break;
        }

        // Checked before anything else claims the line: an unrecognised `@@@` header would
        // otherwise be filed as a header and its body read as loose text, silently truncating
        // the hunk and inventing a file entry out of a `--- removed in both parents` line.
        // Only outside a hunk body, where a leading marker keeps these prefixes unreachable.
        if !st.in_hunk && is_combined_marker(line) {
            return Err(ParseError::Combined(
                String::from_utf8_lossy(line).into_owned(),
            ));
        }

        if let Some(rest) = line.strip_prefix(b"diff --git ") {
            if let Some(f) = cur.take() {
                files.push(f);
            }
            cur = Some(start_git_file(line, rest));
            st = FileState::default();
            continue;
        }

        if starts_plain_file(line, cur.is_some(), &st) {
            if let Some(f) = cur.take() {
                files.push(f);
            }
            cur = Some(new_file(Vec::new()));
            st = FileState::default();
        }

        let Some(f) = cur.as_mut() else {
            // Before the first file entry: the mail head of a format-patch, or any other
            // preamble. Kept in place so the output is the input plus the selection, not a
            // patch that lost its head while keeping its footer.
            preamble.push(line.to_vec());
            continue;
        };

        if line.starts_with(b"@@ ") {
            open_hunk(f, line, &mut st)?;
            continue;
        }

        if st.in_hunk {
            let FileContent::Text(hunks) = &mut f.content else {
                unreachable!("in_hunk is set only after a text hunk header")
            };
            let h = hunks
                .last_mut()
                .expect("in_hunk implies a hunk was already pushed");
            if let BodyLine::HunkEnded = take_body_line(h, line, &mut st.remaining) {
                st.in_hunk = false;
                push_header(f, line);
            }
            continue;
        }

        if line.starts_with(b"+++ ") {
            st.saw_marker_pair = true;
        }
        push_header(f, line);
    }

    if let Some(f) = cur.take() {
        files.push(f);
    }
    Ok(Patch {
        preamble,
        files,
        // `split` yields a trailing empty element exactly when the input ends with a newline,
        // and the loop above skips it; a non-empty final element means the newline was absent.
        no_trailing_newline: !input.is_empty() && !input.ends_with(b"\n"),
    })
}

/// Start a hunk from its `@@` header: record the declared counts and open the body.
fn open_hunk(f: &mut FileDiff, line: &[u8], st: &mut FileState) -> Result<(), ParseError> {
    let hunk = parse_hunk_header(line)?;
    let FileContent::Text(hunks) = &mut f.content else {
        return Err(ParseError::Unexpected("hunk in binary file".into()));
    };
    st.remaining = Remaining {
        old: hunk.old_lines,
        new: hunk.new_lines,
    };
    hunks.push(hunk);
    st.saw_hunk = true;
    // A degenerate hunk declaring zero lines has no body to consume.
    st.in_hunk = !st.remaining.exhausted();
    Ok(())
}

/// An empty file entry with the given already-collected header lines.
fn new_file(headers: Vec<Vec<u8>>) -> FileDiff {
    FileDiff {
        headers,
        trailer: Vec::new(),
        old_path: None,
        new_path: None,
        content: FileContent::Text(Vec::new()),
    }
}

/// Whether the line marks a combined diff: the header git writes for a merge (`diff --cc`,
/// `diff --combined`) or its hunk header, which has one `@` per side plus one (`@@@ -1,3 -1,3
/// +1,3 @@@` for two parents).
pub fn is_combined_marker(line: &[u8]) -> bool {
    line.starts_with(b"diff --cc ")
        || line.starts_with(b"diff --combined ")
        || line.starts_with(b"@@@")
}

/// Open a file entry from a `diff --git a/x b/y` line. The paths are seeded from the command
/// line itself: a binary file, a mode-only change and a pure rename carry no `---`/`+++`, and
/// those lines overwrite this when they do appear.
fn start_git_file(line: &[u8], rest: &[u8]) -> FileDiff {
    let mut f = new_file(vec![line.to_vec()]);
    if let Some((old_path, new_path)) = split_diff_git_paths(rest) {
        f.old_path = Some(old_path);
        f.new_path = Some(new_path);
    }
    f
}

/// Whether `line` opens a file entry in a plain (non-git) diff: a `--- ` line when no file is
/// being built, when the current file's last hunk has consumed all its declared lines (the next
/// file), or when the current file is fully declared but has no hunks at all (a header-only
/// entry, otherwise the next file's markers would overwrite this one's paths). The
/// `remaining.exhausted()` guard is essential: inside a hunk body a deletion line whose content
/// begins with "-- " renders as "--- <text>" and must be consumed as a deletion, not mistaken
/// for a file header.
fn starts_plain_file(line: &[u8], have_file: bool, st: &FileState) -> bool {
    line.starts_with(b"--- ")
        && (!have_file
            || (st.saw_hunk && st.remaining.exhausted())
            || (!st.saw_hunk && st.saw_marker_pair))
}

/// Read one line of a hunk body into `h`, drawing down `rem`.
///
/// A body line belongs to the hunk only while the relevant declared count has budget: context
/// consumes one old and one new, `+` one new, `-` one old. Once a side is exhausted, a further
/// line of that kind is not part of this hunk (the header over-declared or the diff is
/// malformed) — report the hunk as ended rather than appending past the declared size.
fn take_body_line(h: &mut Hunk, line: &[u8], rem: &mut Remaining) -> BodyLine {
    match line.first() {
        Some(b' ') if rem.old > 0 && rem.new > 0 => {
            h.lines.push(mk_line(LineKind::Context, &line[1..]));
            rem.old -= 1;
            rem.new -= 1;
        }
        // A context line for an empty source line is a lone space; transports that strip
        // trailing whitespace deliver it as a zero-length line. `git apply` accepts that, so
        // treat it as context while the counts have budget — the emitted diff restores the
        // marker.
        None if rem.old > 0 && rem.new > 0 => {
            h.lines.push(mk_line(LineKind::Context, b""));
            rem.old -= 1;
            rem.new -= 1;
        }
        Some(b'+') if rem.new > 0 => {
            h.lines.push(mk_line(LineKind::Add, &line[1..]));
            rem.new -= 1;
        }
        Some(b'-') if rem.old > 0 => {
            h.lines.push(mk_line(LineKind::Del, &line[1..]));
            rem.old -= 1;
        }
        _ if line.starts_with(b"\\ ") => {
            // The marker qualifies the line before it, so one before any body line does not
            // belong to this hunk. Report the hunk as ended and let the line be recorded where
            // it stands: dropping it would emit a diff that differs from its input.
            let Some(last) = h.lines.last_mut() else {
                return BodyLine::HunkEnded;
            };
            // Verbatim: in a CRLF diff the marker arrives with its CR, and emitting it with a
            // bare newline would leave the output with mixed line endings.
            last.no_newline = Some(line.to_vec());
        }
        _ => return BodyLine::HunkEnded,
    }
    BodyLine::Consumed
}

/// Position of the first occurrence of `needle` within `hay`.
fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

fn mk_line(kind: LineKind, text: &[u8]) -> Line {
    Line {
        kind,
        text: text.to_vec(),
        no_newline: None,
    }
}

/// Record a line that is not part of a hunk body. Once the file has hunks such a line belongs
/// after them (`trailer`, tagged with how many hunks precede it), not with the leading
/// `headers` — emitting it up front reorders the diff and `git apply` rejects the result.
fn push_header(f: &mut FileDiff, line: &[u8]) {
    // Once the entry is binary, every further line of it belongs to the payload. `git diff
    // --binary` writes `literal <n>`, base85 lines and blank separators after the marker, and
    // none of them look like a marker; treating them as leading headers puts them above the
    // marker on the way out, which git rejects as garbage.
    if let FileContent::Binary(b) = &mut f.content {
        b.push(line.to_vec());
        return;
    }
    let hunks_so_far = f.hunk_count();
    if is_binary_marker(line) {
        match &mut f.content {
            FileContent::Text(h) if h.is_empty() => {
                f.content = FileContent::Binary(vec![line.to_vec()]);
            }
            // A binary marker after hunks is not a valid combination, but the line is still
            // part of the input and is kept in place rather than dropped.
            FileContent::Text(_) => f.trailer.push((hunks_so_far, line.to_vec())),
            FileContent::Binary(_) => unreachable!("handled above"),
        }
        return;
    }
    if hunks_so_far > 0 {
        f.trailer.push((hunks_so_far, line.to_vec()));
        return;
    }
    if let Some(rest) = line.strip_prefix(b"--- ") {
        f.old_path = Some(strip_ab(rest));
    } else if let Some(rest) = line.strip_prefix(b"+++ ") {
        f.new_path = Some(strip_ab(rest));
    }
    f.headers.push(line.to_vec());
}

/// Whether a line announces binary content: either git's summary form (`Binary files ... differ`)
/// or the header of a full binary patch. The trailing CR of a CRLF diff is not part of the
/// marker — without stripping it the marker goes unrecognised and the payload is read as text.
fn is_binary_marker(line: &[u8]) -> bool {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    line.starts_with(b"Binary files ") || line == b"GIT binary patch"
}

/// Decode a path as written after `--- `/`+++ ` or on a `diff --git` line: undo git's quoting
/// of names that need escaping (`"a/\303\251.txt"` — the `core.quotePath` default for
/// non-ASCII), drop a trailing tab-and-timestamp, strip a leading `a/` or `b/`, and drop the
/// CR a CRLF diff leaves at the end of the header line. The result is the file's real bytes,
/// which is what a `path:` selector is matched against; `headers` keep the original spelling
/// so the emitted diff is unchanged.
fn strip_ab(s: &[u8]) -> Vec<u8> {
    let s = s.strip_suffix(b"\r").unwrap_or(s);
    if let Some(decoded) = unquote(s) {
        return strip_ab_prefix(&decoded).to_vec();
    }
    let s = match s.iter().position(|&b| b == b'\t') {
        Some(i) => &s[..i],
        None => s,
    };
    strip_ab_prefix(s).to_vec()
}

fn strip_ab_prefix(s: &[u8]) -> &[u8] {
    s.strip_prefix(b"a/")
        .or_else(|| s.strip_prefix(b"b/"))
        .unwrap_or(s)
}

/// Decode git's quoted path form. `None` when `s` is not a quoted string or its escapes are
/// malformed — the caller then treats the bytes as a literal name.
fn unquote(s: &[u8]) -> Option<Vec<u8>> {
    let body = s.strip_prefix(b"\"")?.strip_suffix(b"\"")?;
    let mut out = Vec::with_capacity(body.len());
    let mut it = body.iter().copied();
    while let Some(b) = it.next() {
        if b != b'\\' {
            out.push(b);
            continue;
        }
        match it.next()? {
            b'a' => out.push(0x07),
            b'b' => out.push(0x08),
            b'f' => out.push(0x0c),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(0x0b),
            b'\\' => out.push(b'\\'),
            b'"' => out.push(b'"'),
            d @ b'0'..=b'7' => {
                // Git writes a non-ASCII byte as a three-digit octal escape.
                let mut v = u32::from(d - b'0');
                for _ in 0..2 {
                    let n = it.next()?;
                    if !n.is_ascii_digit() || n > b'7' {
                        return None;
                    }
                    v = v * 8 + u32::from(n - b'0');
                }
                out.push(u8::try_from(v).ok()?);
            }
            _ => return None,
        }
    }
    Some(out)
}

/// Split the two paths on a `diff --git ` line (the part after the command). A binary file,
/// a mode-only change and a pure rename have no `---`/`+++` lines, so this is the only place
/// their name appears. Git quotes a name that needs escaping, which makes the first path
/// self-delimiting; otherwise both names are the same in all but renames, so the midpoint
/// split is exact, and the last ` b/` is the fallback for the rename case.
fn split_diff_git_paths(rest: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let rest = rest.strip_suffix(b"\r").unwrap_or(rest);
    if rest.first() == Some(&b'"') {
        let mut end = None;
        let mut escaped = false;
        for (i, &b) in rest.iter().enumerate().skip(1) {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => {
                    end = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let end = end?;
        let second = rest.get(end + 2..)?;
        return Some((strip_ab(&rest[..=end]), strip_ab(second)));
    }
    let mid = rest.len() / 2;
    if rest.len() % 2 == 1 && rest.get(mid) == Some(&b' ') {
        return Some((strip_ab(&rest[..mid]), strip_ab(&rest[mid + 1..])));
    }
    let at = find_last_subslice(rest, b" b/")?;
    Some((strip_ab(&rest[..at]), strip_ab(&rest[at + 1..])))
}

/// Position of the last occurrence of `needle` within `hay`.
fn find_last_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).rposition(|w| w == needle)
}

fn parse_hunk_header(line: &[u8]) -> Result<Hunk, ParseError> {
    // Format: @@ -os[,ol] +ns[,nl] @@[ section]
    let bad = || ParseError::BadHunkHeader(String::from_utf8_lossy(line).into_owned());
    // The ` @@` that closes the range portion of the header; its length drives the offset of the
    // section text that follows.
    const SEP: &[u8] = b" @@";
    let body = line.strip_prefix(b"@@ ").ok_or_else(bad)?;
    let end = find_subslice(body, SEP).ok_or_else(bad)?;
    let ranges = &body[..end];
    let after = &body[end + SEP.len()..];
    let section = after.strip_prefix(b" ").unwrap_or(after).to_vec();
    // The ranges portion (`-os,ol +ns,nl`) is ASCII for any valid hunk header.
    let ranges = std::str::from_utf8(ranges).map_err(|_| bad())?;
    let mut it = ranges.split_whitespace();
    let old = it.next().ok_or_else(bad)?;
    let new = it.next().ok_or_else(bad)?;
    // A third token is not a header hunkpick can represent. Ignoring it parsed the line as if
    // it read differently and emitted it without those bytes — a silent rewrite of the input,
    // at exit 0. The same holds for a third component inside a range (see `parse_range`).
    if it.next().is_some() {
        return Err(bad());
    }
    let (old_start, old_lines) = parse_range(old.strip_prefix('-').unwrap_or(old))?;
    let (new_start, new_lines) = parse_range(new.strip_prefix('+').unwrap_or(new))?;
    Ok(Hunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
        section,
        lines: Vec::new(),
    })
}

fn parse_range(s: &str) -> Result<(u32, u32), ParseError> {
    let mut parts = s.split(',');
    let start = parts
        .next()
        .and_then(|x| x.parse().ok())
        .ok_or_else(|| ParseError::BadHunkHeader(s.to_string()))?;
    let count = match parts.next() {
        Some(c) => c
            .parse()
            .map_err(|_| ParseError::BadHunkHeader(s.to_string()))?,
        None => 1,
    };
    // `-1,3,9` has no meaning in a unified diff; parsing it as `-1,3` would drop the rest on
    // the way out.
    if parts.next().is_some() {
        return Err(ParseError::BadHunkHeader(s.to_string()));
    }
    Ok((start, count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emit::emit;
    use crate::model::{FileContent, LineKind};

    const ONE: &str = "\
diff --git a/f.txt b/f.txt
index 111..222 100644
--- a/f.txt
+++ b/f.txt
@@ -1,3 +1,3 @@
 a
-b
+B
 c
";

    #[test]
    fn parses_single_hunk() {
        let p = parse(ONE.as_bytes()).unwrap();
        assert_eq!(p.files.len(), 1);
        let f = &p.files[0];
        assert_eq!(f.old_path.as_deref(), Some(b"f.txt".as_slice()));
        assert_eq!(f.new_path.as_deref(), Some(b"f.txt".as_slice()));
        let FileContent::Text(hunks) = &f.content else {
            panic!("text")
        };
        assert_eq!(hunks.len(), 1);
        let h = &hunks[0];
        assert_eq!(
            (h.old_start, h.old_lines, h.new_start, h.new_lines),
            (1, 3, 1, 3)
        );
        assert_eq!(h.lines.len(), 4);
        assert_eq!(h.lines[1].kind, LineKind::Del);
        assert_eq!(h.lines[1].text.as_slice(), b"b");
    }

    #[test]
    fn parses_multi_hunk_with_section() {
        let src = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,2 +1,2 @@ fn one()
 x
-y
+Y
@@ -10,2 +10,3 @@ fn two()
 p
+q
 r
";
        let p = parse(src.as_bytes()).unwrap();
        let FileContent::Text(h) = &p.files[0].content else {
            panic!()
        };
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].section.as_slice(), b"fn one()");
        assert_eq!(h[1].section.as_slice(), b"fn two()");
        assert_eq!((h[1].new_start, h[1].new_lines), (10, 3));
    }

    #[test]
    fn parses_multi_file() {
        let src = "\
diff --git a/x b/x
--- a/x
+++ b/x
@@ -1 +1 @@
-1
+2
diff --git a/y b/y
--- a/y
+++ b/y
@@ -1 +1 @@
-3
+4
";
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(p.files.len(), 2);
        assert_eq!(p.files[0].new_path.as_deref(), Some(b"x".as_slice()));
        assert_eq!(p.files[1].new_path.as_deref(), Some(b"y".as_slice()));
    }

    #[test]
    fn parses_no_newline_marker() {
        let src = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1 +1 @@
-old
\\ No newline at end of file
+new
\\ No newline at end of file
";
        let p = parse(src.as_bytes()).unwrap();
        let FileContent::Text(h) = &p.files[0].content else {
            panic!()
        };
        assert!(h[0].lines[0].no_newline.is_some());
        assert!(h[0].lines[1].no_newline.is_some());
    }

    #[test]
    fn parses_binary_file() {
        let src = "\
diff --git a/img.png b/img.png
index 111..222 100644
Binary files a/img.png and b/img.png differ
";
        let p = parse(src.as_bytes()).unwrap();
        assert!(matches!(p.files[0].content, FileContent::Binary(_)));
    }

    #[test]
    fn deletion_line_dash_dash_not_mistaken_for_file_header() {
        // A deletion whose content starts with "-- " renders as "--- <text>";
        // inside a hunk body it must be consumed as a deletion, not a new file.
        let src = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,2 +1,2 @@
 xyz
--- old comment
+++ new comment
";
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(p.files.len(), 1, "no phantom file");
        let FileContent::Text(h) = &p.files[0].content else {
            panic!()
        };
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].lines.len(), 3);
        assert_eq!(h[0].lines[1].kind, LineKind::Del);
        assert_eq!(h[0].lines[1].text.as_slice(), b"-- old comment");
        assert_eq!(h[0].lines[2].kind, LineKind::Add);
        assert_eq!(h[0].lines[2].text.as_slice(), b"++ new comment");
    }

    #[test]
    fn empty_line_in_hunk_body_is_a_context_line() {
        // A context line for an empty source line is " " (marker plus nothing). Transports
        // that strip trailing whitespace turn it into a zero-length line; `git apply` still
        // accepts such a diff, so the body must continue rather than end here.
        let src = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,6 +1,6 @@
 a
-b
+B
 c
-d
+D

-x
+X
";
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(p.files.len(), 1, "no phantom file");
        let f = &p.files[0];
        assert_eq!(f.headers.len(), 3, "body lines must not leak into headers");
        let FileContent::Text(h) = &f.content else {
            panic!()
        };
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].lines.len(), 9, "whole body belongs to the hunk");
        assert_eq!(h[0].lines[6].kind, LineKind::Context);
        assert!(h[0].lines[6].text.is_empty());
        assert_eq!(h[0].lines[7].kind, LineKind::Del);
        assert_eq!(h[0].lines[7].text.as_slice(), b"x");
    }

    #[test]
    fn empty_line_past_the_declared_count_still_ends_the_hunk() {
        // Once the declared counts are exhausted an empty line is not a body line: it is
        // whatever follows the hunk, and is recorded after it — not among the leading headers,
        // which would move it above the first `@@` on output.
        let src = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1 +1 @@
-a
+A

";
        let p = parse(src.as_bytes()).unwrap();
        let f = &p.files[0];
        let FileContent::Text(h) = &f.content else {
            panic!()
        };
        assert_eq!(h[0].lines.len(), 2, "only the declared body lines");
        assert_eq!(
            f.headers.len(),
            3,
            "no body-adjacent line among the headers"
        );
        assert_eq!(f.trailer, vec![(1usize, b"".to_vec())]);
    }

    #[test]
    fn binary_marker_after_hunks_is_kept_not_dropped() {
        // A binary marker never legitimately follows hunks, but silently swallowing the line
        // would emit a diff that differs from its input with exit code 0.
        let src = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1 +1 @@
-a
+A
Binary files a/f and b/f differ
";
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(
            p.files[0].trailer,
            vec![(1usize, b"Binary files a/f and b/f differ".to_vec())]
        );
    }

    #[test]
    fn a_format_patch_mail_header_survives_the_round_trip() {
        // `git format-patch` wraps the diff in a mail: headers, the commit message and a
        // diffstat come before the first `diff --git`, the `-- ` signature after the last hunk.
        // Keeping the signature while dropping the head would leave a patch `git am` no longer
        // accepts, with the mail's footer still attached.
        let src = concat!(
            "From 0000000000000000000000000000000000000000 Mon Sep 17 00:00:00 2001\n",
            "From: Someone <someone@example.invalid>\n",
            "Subject: [PATCH] change f\n",
            "\n",
            " f | 2 +-\n",
            " 1 file changed, 1 insertion(+), 1 deletion(-)\n",
            "\n",
            "diff --git a/f b/f\n",
            "--- a/f\n",
            "+++ b/f\n",
            "@@ -1 +1 @@\n",
            "-a\n",
            "+A\n",
            "-- \n",
            "2.53.0\n",
        );
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(emit(&p), src.as_bytes());
    }

    #[test]
    fn a_no_newline_marker_before_any_body_line_is_kept() {
        // The marker belongs to the line before it, so a body that starts with one is
        // malformed. Swallowing the line would emit a diff that differs from its input with
        // exit code 0; it stays where it was, and the empty hunk is what the input check
        // reports.
        let src = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1 +1 @@
\\ No newline at end of file
";
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(emit(&p), src.as_bytes(), "no byte may be dropped");
    }

    #[test]
    fn crlf_binary_marker_still_starts_a_binary_entry() {
        // The CR belongs to the line ending, not to the marker. Missing that reads the payload
        // as text and reorders it on the way out.
        let src = "\
diff --git a/f.bin b/f.bin\r
index 34b631e..f0c6ea3 100644\r
GIT binary patch\r
literal 13\r
UcmeAS@N;M2<Y3P)$w(~%02liMsQ>@~\r
";
        let p = parse(src.as_bytes()).unwrap();
        let FileContent::Binary(b) = &p.files[0].content else {
            panic!("a CRLF binary marker must start a binary entry");
        };
        assert_eq!(b.len(), 3, "marker and payload belong to the entry");
        assert_eq!(emit(&p), src.as_bytes(), "and come back unchanged");
    }

    #[test]
    fn signature_after_the_last_hunk_keeps_its_place() {
        // `git format-patch` ends a patch with "-- " and the git version. Both lines follow
        // the last hunk and must stay there.
        // Spelled with concat! so the trailing space of the "-- " marker stays visible.
        let src = concat!(
            "diff --git a/f b/f\n",
            "--- a/f\n",
            "+++ b/f\n",
            "@@ -1 +1 @@\n",
            "-a\n",
            "+A\n",
            "-- \n",
            "2.53.0\n",
        );
        let p = parse(src.as_bytes()).unwrap();
        let f = &p.files[0];
        assert_eq!(
            f.trailer,
            vec![(1usize, b"-- ".to_vec()), (1usize, b"2.53.0".to_vec())]
        );
    }

    #[test]
    fn plain_diff_entry_without_hunks_does_not_absorb_the_next_file() {
        // A header-only entry in a plain diff is complete once it has both marker lines;
        // the next "--- " opens another file instead of overwriting this one's paths.
        let src = "\
--- a/x
+++ b/x
--- a/y
+++ b/y
@@ -1 +1 @@
-1
+2
";
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(p.files.len(), 2, "two separate entries");
        assert_eq!(p.files[0].new_path.as_deref(), Some(b"x".as_slice()));
        assert_eq!(p.files[1].new_path.as_deref(), Some(b"y".as_slice()));
    }

    #[test]
    fn quoted_path_is_decoded_to_its_bytes() {
        // With core.quotePath at its default git writes a non-ASCII name quoted and
        // C-escaped: `--- "a/\303\251.txt"`. The stored path must be the real bytes, so a
        // selector spelled with the actual file name matches.
        let src = "\
diff --git \"a/\\303\\251.txt\" \"b/\\303\\251.txt\"
--- \"a/\\303\\251.txt\"
+++ \"b/\\303\\251.txt\"
@@ -1 +1 @@
-a
+A
";
        let p = parse(src.as_bytes()).unwrap();
        let f = &p.files[0];
        assert_eq!(f.old_path.as_deref(), Some("é.txt".as_bytes()));
        assert_eq!(f.new_path.as_deref(), Some("é.txt".as_bytes()));
        assert_eq!(f.display_path(), "é.txt");
    }

    #[test]
    fn quoted_path_keeps_escaped_specials() {
        // A quoted name may also carry \\ and \" and control escapes; all decode to bytes.
        let src = "\
--- \"a/we\\\"ird\\tname\"
+++ \"b/we\\\"ird\\tname\"
@@ -1 +1 @@
-a
+A
";
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(
            p.files[0].new_path.as_deref(),
            Some(b"we\"ird\tname".as_slice())
        );
    }

    #[test]
    fn crlf_diff_path_has_no_carriage_return() {
        // A diff with CRLF endings leaves \r at the end of the header line; it is part of
        // the line ending, not of the file name.
        let src = "diff --git a/f b/f\r\n--- a/f\r\n+++ b/f\r\n@@ -1 +1 @@\r\n-a\r\n+A\r\n";
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(p.files[0].new_path.as_deref(), Some(b"f".as_slice()));
        assert_eq!(p.files[0].old_path.as_deref(), Some(b"f".as_slice()));
    }

    #[test]
    fn binary_file_path_comes_from_the_diff_git_line() {
        // A binary file has no ---/+++ lines, so its name is only in `diff --git`.
        let src = "\
diff --git a/img.png b/img.png
index 111..222 100644
Binary files a/img.png and b/img.png differ
";
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(p.files[0].display_path(), "img.png");
    }

    #[test]
    fn diff_git_paths_do_not_override_the_marker_lines() {
        // A rename states both names; ---/+++ are authoritative when present.
        let src = "\
diff --git a/old b/new
similarity index 90%
rename from old
rename to new
--- a/old
+++ b/new
@@ -1 +1 @@
-a
+A
";
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(p.files[0].old_path.as_deref(), Some(b"old".as_slice()));
        assert_eq!(p.files[0].new_path.as_deref(), Some(b"new".as_slice()));
    }

    #[test]
    fn parses_plain_non_git_diff() {
        let src = "\
--- old.txt\t2020-01-01
+++ new.txt\t2020-01-02
@@ -1 +1 @@
-a
+b
";
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(p.files.len(), 1);
        assert_eq!(p.files[0].old_path.as_deref(), Some(b"old.txt".as_slice()));
        assert!(!p.files[0].headers.is_empty());
    }
}
