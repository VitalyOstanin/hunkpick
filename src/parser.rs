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

/// Whether `input` has a line that opens a diff.
///
/// The set of lines that count lives here, next to the code that reads them: a caller deciding
/// whether to hand something to [`parse`] at all — the CLI does, before it reports "this is not
/// a diff" — must not keep a second list that can disagree with this one.
///
/// A combined diff counts: [`parse`] rejects it by name, which is a better answer than "no diff
/// markers found". Its `---`/`+++` pair is omitted for a file resolved the same way in both
/// parents, so the ordinary markers do not always appear in one.
pub fn looks_like_a_diff(input: &[u8]) -> bool {
    input
        .split(|&b| b == b'\n')
        .any(|line| diff_marker(line).is_some())
}

/// The marker at the head of `line` that opens a diff, if one does — the same set
/// [`looks_like_a_diff`] answers on, reported rather than counted.
///
/// Which marker it is decides how the rest of the line is read, so a caller that has to weigh a
/// line against its marker — [`says_what_its_marker_promises`] does — starts here rather than
/// keeping a second list beside this one.
fn diff_marker(line: &[u8]) -> Option<&'static [u8]> {
    longest_prefix(markers(), line)
}

/// Every marker a diff line may open with, ordinary and combined.
///
/// Not part of the library contract, and public for the reason the lists themselves are not:
/// the binary is a separate crate, and a test of it that reads one marker closely and the rest
/// loosely — every revision of the UTF-16 check so far has — would otherwise write the set out a
/// second time and be free to fall behind this one.
#[doc(hidden)]
pub fn markers() -> impl Iterator<Item = &'static [u8]> {
    ORDINARY_MARKERS
        .iter()
        .chain(COMBINED_MARKERS.iter())
        .copied()
}

/// The longest of `prefixes` that opens `line`.
///
/// The longest match, not the first: what a prefix leaves behind is read past its length, so a
/// prefix that opens another would otherwise let the order the two are listed in decide how much
/// of the line counts as said beyond it. Stated over a list handed in rather than over the
/// arrays directly — the markers are one such list and the headers ASCII follows another, and
/// no member of either opens another today, so the rule can only be exercised against a pair
/// where one does.
fn longest_prefix(
    prefixes: impl Iterator<Item = &'static [u8]>,
    line: &[u8],
) -> Option<&'static [u8]> {
    prefixes
        .filter(|p| line.starts_with(p))
        .max_by_key(|p| p.len())
}

/// Whether `line` carries what the marker at its head promises: a `@@` line that parses as a
/// hunk header, and for any other marker something past it that is not whitespace.
///
/// A marker on its own says nothing about the stream it was read out of. Binary data holding
/// `@@ ` or `diff --git ` NUL-padded carries one, and so does a line that is the marker and the
/// line ending a patch written under Windows leaves behind. A caller weighing such a line — the
/// CLI does, judging whether what it read out of a stream with no byte-order mark is evidence of
/// UTF-16 — asks about the diff format, so the answer is written next to the code that reads it
/// rather than as a length or a threshold at the call site.
fn says_what_its_marker_promises(line: &[u8]) -> bool {
    let Some(marker) = diff_marker(line) else {
        return false;
    };
    // A hunk header is the one marker line whose shape the parser already states in full, and
    // the shape is what tells `@@ -1` inside binary data from a header a diff carries.
    if marker == HUNK_HEADER_MARKER {
        return parse_hunk_header(line).is_ok();
    }
    carries_something_past(line, marker)
}

/// Whether `line` says anything past `prefix` beyond whitespace.
///
/// The one reading given to both lists a line is weighed against — the markers and the headers
/// ASCII follows — rather than the same three lines written beside each: the rule would
/// otherwise be free to change for a marker and stay as it was for a header, and
/// [`reads_as_a_diff_line`] would answer the same tail two ways.
///
/// Whitespace as the language already spells it, rather than a set written out here beside a
/// sentence naming its members: the two would be free to disagree. A line ending counts as
/// whitespace wherever the caller read the line from — one that splits on `\n` never sees it,
/// one that hands the line over whole does, and the answer is the same either way.
///
/// `prefix` is expected to open `line`: the tail is what stands past it, which is a reading only
/// there. Asked where it does not, the answer is that nothing stands past it — the helper is
/// asked by a caller holding a line of its own and a prefix chosen by a separate reading, and an
/// answer is of more use there than a stop. The head of the line is read rather than the length
/// of the prefix measured off it: a prefix longer than the line would otherwise be the only one
/// refused, and a shorter one that opens nothing would hand back a tail cut at a place no
/// reading pointed at.
///
/// Not part of the library contract, and public for the reason [`markers`] is: the binary is a
/// separate crate, and its UTF-16 check weighs a header a path follows against the line it read
/// the header out of. Written out there, "something past it" would be a second reading of the
/// same tail, free to keep whitespace where this one stopped counting it.
#[doc(hidden)]
pub fn carries_something_past(line: &[u8], prefix: &[u8]) -> bool {
    line.strip_prefix(prefix)
        .is_some_and(|tail| tail.iter().any(|b| !b.is_ascii_whitespace()))
}

/// The lines a diff writes about a file besides the markers that open an entry or a hunk, and
/// that a path follows. A rename and a copy carry no `---`, `+++` or `@@`, so past the
/// `diff --git` line these are all such an entry has; unlike a marker they promise nothing
/// after themselves, because what follows is a path, which may be in any script and is not
/// there to be read.
const HEADERS_A_PATH_FOLLOWS: [&[u8]; 4] =
    [b"rename from ", b"rename to ", b"copy from ", b"copy to "];

/// The other lines a diff writes about a file: an object name, a mode, a percentage. What
/// follows each is ASCII wherever the patch was written, so they are weighed the way a marker is
/// rather than taken on their own head — a bare `index ` says as little about the stream it was
/// read out of as a bare `--- ` does.
const HEADERS_ASCII_FOLLOWS: [&[u8]; 7] = [
    b"index ",
    b"similarity index ",
    b"dissimilarity index ",
    b"new file mode ",
    b"deleted file mode ",
    b"old mode ",
    b"new mode ",
];

/// Whether `line` reads as a line a diff writes: a marker line carrying what its marker
/// promises, an extended header carrying what stands after it, or one of the four headers a
/// path follows, which is read on its own head because the path may be in any script.
///
/// One such line is not evidence that the stream around it is a diff — a NUL-padded `--- x` and
/// a NUL-padded `@@ -1` are equally an accident of the bytes a PNG happens to hold. Two of them
/// are, and a caller judging a stream counts rather than weighs: every attempt to make a single
/// line decisive has had to say what a diff may not open with, and each such rule refused a
/// patch someone writes — a mail whose first header is not ASCII, a patch opening with a blank
/// line, a path in another script.
///
/// Counting is only as strong as what it counts: a line taken on its head where its head is all
/// there is to read hands binary data two lines as readily as a patch, so a header is taken that
/// way exactly where reading further is not an option. Where it is an option the caller takes
/// it: one holding the line this was read out of can ask, with
/// [`the_header_a_path_follows_in`] and [`carries_something_past`], that the path be there — the
/// binary's UTF-16 check does, since a line read no further than its first unit that is not
/// ASCII leaves a bare header and a header a path follows spelled the same.
pub fn reads_as_a_diff_line(line: &[u8]) -> bool {
    says_what_its_marker_promises(line)
        || the_header_a_path_follows_in(line).is_some()
        || says_what_its_header_promises(line)
}

/// The header a path follows that opens `line`, the longest of them where one opens another.
///
/// Not part of the library contract, and public for the reason [`markers`] is: the binary's
/// UTF-16 check weighs such a header against the line it read it out of, and needs to know
/// which header that is. Looked up here rather than there, so that the list is read by one rule
/// — a lookup written out beside the check would be free to take the first that opens the line,
/// and the length of what it found is what decides how much of the line counts as said past it.
#[doc(hidden)]
pub fn the_header_a_path_follows_in(line: &[u8]) -> Option<&'static [u8]> {
    longest_prefix(headers_a_path_follows(), line)
}

/// Every extended header a path follows, the ones read on their own head.
///
/// Not part of the library contract, and public for the reason [`markers`] is: the binary is a
/// separate crate, and its cases weighing what the count of two rests on have to name the
/// headers they try. Written out there, the set answered for whichever of the four happened to
/// be named and was free to stay that way as the list grew. The check itself asks
/// [`the_header_a_path_follows_in`], which reads this list by the rule the parser reads it by.
#[doc(hidden)]
pub fn headers_a_path_follows() -> impl Iterator<Item = &'static [u8]> {
    HEADERS_A_PATH_FOLLOWS.iter().copied()
}

/// Every extended header whose line is ASCII past it, so that a caller weighing one is weighing
/// the same set the parser does.
///
/// Not part of the library contract, and public for the reason [`markers`] is: the binary is a
/// separate crate, and its test that two bare headers say nothing has to name the headers it
/// tries. Written out there, the set answered for three of the seven and was free to stay that
/// way as the list grew.
#[doc(hidden)]
pub fn headers_ascii_follows() -> impl Iterator<Item = &'static [u8]> {
    HEADERS_ASCII_FOLLOWS.iter().copied()
}

/// Whether `line` opens with a header that ASCII follows and carries something past it that is
/// not whitespace — the reading [`says_what_its_marker_promises`] gives a marker, given to the
/// headers it can be given to.
fn says_what_its_header_promises(line: &[u8]) -> bool {
    longest_prefix(headers_ascii_follows(), line)
        .is_some_and(|header| carries_something_past(line, header))
}

/// The line that opens a hunk, named apart from the list it belongs to: it is the one marker
/// whose line the parser reads in full, so [`says_what_its_marker_promises`] branches on it, and
/// a marker written out a second time there would leave that branch behind if this one changed.
const HUNK_HEADER_MARKER: &[u8] = b"@@ ";

/// The lines that open a file entry or a hunk in an ordinary diff.
const ORDINARY_MARKERS: [&[u8]; 5] = [
    b"diff --git ",
    b"--- ",
    b"+++ ",
    HUNK_HEADER_MARKER,
    b"Binary files ",
];

/// The lines that mark a combined diff: the header git writes for a merge (`diff --cc`,
/// `diff --combined`) and its hunk header, which has one `@` per side plus one (`@@@ -1,3 -1,3
/// +1,3 @@@` for two parents).
const COMBINED_MARKERS: [&[u8]; 3] = [b"diff --cc ", b"diff --combined ", b"@@@"];

/// Whether the line marks a combined diff.
fn is_combined_marker(line: &[u8]) -> bool {
    COMBINED_MARKERS.iter().any(|m| line.starts_with(m))
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
    // Split on the one ASCII space the format prescribes, not on `char::is_whitespace`:
    // `split_whitespace` also cuts on tabs and on non-ASCII code points such as U+00A0, and
    // `emit` rebuilds the header with a plain space — the separator bytes would be dropped on
    // the way out, a silent rewrite of the input at exit 0 (the same reason a third token is
    // refused below). git calls such a header a corrupt patch; hunkpick must not launder it
    // into one git accepts.
    let mut it = ranges.split(' ');
    let old = it.next().ok_or_else(bad)?;
    let new = it.next().ok_or_else(bad)?;
    // A third token is not a header hunkpick can represent. Ignoring it parsed the line as if
    // it read differently and emitted it without those bytes — a silent rewrite of the input,
    // at exit 0. The same holds for a third component inside a range (see `parse_range`).
    if it.next().is_some() {
        return Err(bad());
    }
    // The sign is required, not merely tolerated: `strip_prefix(..).unwrap_or(token)` made a
    // missing one indistinguishable from a present one, and `str::parse::<u32>` accepts a
    // leading `+` on top of that. `@@ +1,3 +1,3 @@`, `@@ -1,3 1,3 @@`, `@@ -+1,3 +1,3 @@` and
    // `@@ -1,+3 +1,3 @@` all parsed and came back out as `@@ -1,3 +1,3 @@` — four more ways to
    // turn a diff git calls a corrupt patch into one it accepts, at exit 0.
    let old = old.strip_prefix('-').ok_or_else(bad)?;
    let new = new.strip_prefix('+').ok_or_else(bad)?;
    let (old_start, old_lines) = parse_range(old)?;
    let (new_start, new_lines) = parse_range(new)?;
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
    let bad = || ParseError::BadHunkHeader(s.to_string());
    let mut parts = s.split(',');
    let start = parts.next().and_then(count_from).ok_or_else(bad)?;
    let count = match parts.next() {
        Some(c) => count_from(c).ok_or_else(bad)?,
        None => 1,
    };
    // `-1,3,9` has no meaning in a unified diff; parsing it as `-1,3` would drop the rest on
    // the way out.
    if parts.next().is_some() {
        return Err(bad());
    }
    Ok((start, count))
}

/// One component of a range: plain ASCII digits, nothing else. `str::parse::<u32>` also takes a
/// leading `+`, which would let `-1,+3` through and render it back as `-1,3` — the input
/// rewritten on the way out.
fn count_from(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emit::emit;
    use crate::model::{FileContent, LineKind};

    /// The longest prefix that opens the line is the one reported, whichever order the prefixes
    /// are listed in. No member of either list the parser weighs a line against opens another
    /// today, so the rule has nothing to bite on there — and a rule nothing exercises is one
    /// nobody finds out has stopped working. Given a pair where one prefix does open the other,
    /// the reading is checked directly.
    #[test]
    fn the_longest_prefix_that_opens_a_line_is_the_one_reported() {
        let prefixes: [&'static [u8]; 2] = [b"@@ ", b"@@ -"];
        let line = b"@@ -1 +1 @@";
        assert_eq!(
            longest_prefix(prefixes.into_iter(), line),
            Some(b"@@ -".as_slice())
        );
        assert_eq!(
            longest_prefix(prefixes.into_iter().rev(), line),
            Some(b"@@ -".as_slice())
        );
    }

    /// A tail is what stands past the prefix, which is a reading only where the prefix opens the
    /// line. Asked where it does not, the answer is that nothing stands past it: a prefix the
    /// line does not carry carries nothing of the line with it.
    ///
    /// Asked of a prefix longer than the line and of one shorter than it: read by length alone,
    /// the first is refused and the second hands back a tail measured off a prefix that is not
    /// there.
    #[test]
    fn a_prefix_that_does_not_open_a_line_carries_nothing() {
        assert!(!carries_something_past(b"@@", b"@@ -1 +1 @@"));
        assert!(!carries_something_past(b"index 100", b"--- "));
    }

    /// What a marker promises is a path or a header, and no arrangement of whitespace is either.
    /// A caller that hands over a line whole passes the line ending the stream wrote with it, and
    /// counting that ending as content past the marker makes every bare marker line say
    /// something — the case this rule exists to refuse.
    ///
    /// Asked of both lists a line is weighed against, and of each of their members, through the
    /// one reading a caller has: the rule is one, and a rule that held for a marker while a
    /// header kept the reading it had would let [`reads_as_a_diff_line`] answer the same tail
    /// two ways. Asking the two lists at two levels would leave that promise resting on this
    /// comment.
    #[test]
    fn a_prefix_followed_only_by_whitespace_promises_nothing() {
        for tail in ["\n", "\r\n", "\x0c", "\t", " "] {
            for marker in markers() {
                let line = [marker, tail.as_bytes()].concat();
                let bare = String::from_utf8_lossy(marker);
                assert!(!reads_as_a_diff_line(&line), "`{bare}` and {tail:?}");
            }
            for header in headers_ascii_follows() {
                let line = [header, tail.as_bytes()].concat();
                let bare = String::from_utf8_lossy(header);
                assert!(!reads_as_a_diff_line(&line), "`{bare}` and {tail:?}");
            }
        }
    }

    /// A header whose line is ASCII past it — an object name, a mode, a percentage — is read the
    /// way a marker is: a bare `index ` says as little about the stream it was read out of as a
    /// bare `--- ` does, and two of them are what binary data holds as readily as a patch. Only
    /// the headers a path follows stand on their own head, because a path may be in any script
    /// and is not there to be read.
    #[test]
    fn a_bare_header_says_something_only_where_a_path_follows_it() {
        for header in headers_a_path_follows() {
            let bare = String::from_utf8_lossy(header);
            assert!(reads_as_a_diff_line(header), "bare `{bare}`");
        }
        for header in headers_ascii_follows() {
            let bare = String::from_utf8_lossy(header);
            assert!(!reads_as_a_diff_line(header), "bare `{bare}`");
            assert!(
                !reads_as_a_diff_line(&[header, b"\r\n".as_slice()].concat()),
                "`{bare}` and the line ending a patch written under Windows leaves"
            );
            assert!(
                reads_as_a_diff_line(&[header, b"1".as_slice()].concat()),
                "`{bare}1`"
            );
        }
        // The lines git writes, beside the tail each header is asked about above: what a header
        // carries in a patch is longer than the one byte the loop appends, and a rule read off
        // the shortest line that passes is a rule no patch was ever weighed against.
        assert!(reads_as_a_diff_line(b"rename from f.txt"));
        assert!(reads_as_a_diff_line(b"similarity index 100%"));
        assert!(reads_as_a_diff_line(b"index 111..222 100644"));
        assert!(!reads_as_a_diff_line(b"renamed the file"));
    }

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
    fn a_non_ascii_space_between_the_ranges_is_rejected() {
        // `str::split_whitespace` cuts on every `White_Space` code point, so a header separated
        // by one of them parsed, and `emit` then rebuilt the header from the parsed numbers with
        // an ASCII space — the input rewritten on the way out, at exit 0. git rejects such a
        // header outright (`corrupt patch`), so hunkpick must not turn it into one git accepts.
        for sep in ["\u{a0}", "\u{85}", "\u{2008}", "\u{3000}"] {
            let src = format!("--- a/f\n+++ b/f\n@@ -1,3{sep}+1,3 @@\n a\n-b\n+B\n c\n");
            assert!(
                matches!(parse(src.as_bytes()), Err(ParseError::BadHunkHeader(_))),
                "separator {sep:?} must not parse"
            );
        }
    }

    #[test]
    fn a_tab_or_a_double_space_between_the_ranges_is_rejected() {
        // Same class as the non-ASCII separator: the exact bytes cannot be reproduced by `emit`,
        // which always writes one space, so accepting them would rewrite the input silently.
        for sep in ["\t", "  "] {
            let src = format!("--- a/f\n+++ b/f\n@@ -1,3{sep}+1,3 @@\n a\n-b\n+B\n c\n");
            assert!(
                matches!(parse(src.as_bytes()), Err(ParseError::BadHunkHeader(_))),
                "separator {sep:?} must not parse"
            );
        }
    }

    #[test]
    fn a_range_without_its_sign_or_with_a_stray_one_is_rejected() {
        // The sign used to be stripped with `unwrap_or(token)`, so its absence read the same as
        // its presence, and `str::parse::<u32>` took a leading `+` on top of that. Every header
        // below came back out as `@@ -1,3 +1,3 @@` at exit 0, while git reads all four as a
        // corrupt patch (or as garbage).
        for header in [
            "@@ +1,3 +1,3 @@",
            "@@ -1,3 1,3 @@",
            "@@ -+1,3 +1,3 @@",
            "@@ -1,+3 +1,3 @@",
        ] {
            let src = format!("--- a/f\n+++ b/f\n{header}\n a\n-b\n+B\n c\n");
            assert!(
                matches!(parse(src.as_bytes()), Err(ParseError::BadHunkHeader(_))),
                "header {header:?} must not parse"
            );
        }
    }

    #[test]
    fn non_ascii_digits_in_a_range_are_rejected() {
        // `str::parse::<u32>` takes ASCII digits only; lock that in so a future move to a
        // hand-rolled parser cannot start accepting `١٢` and emitting `12`.
        let src = "--- a/f\n+++ b/f\n@@ -\u{661},3 +1,3 @@\n a\n-b\n+B\n c\n";
        assert!(matches!(
            parse(src.as_bytes()),
            Err(ParseError::BadHunkHeader(_))
        ));
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
