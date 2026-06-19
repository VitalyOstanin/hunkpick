use crate::model::*;

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    BadHunkHeader(String),
    Unexpected(String),
}

pub fn parse(input: &[u8]) -> Result<Patch, ParseError> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut cur: Option<FileDiff> = None;
    let mut in_hunk = false;

    let mut lines = input.split(|&b| b == b'\n').peekable();
    while let Some(line) = lines.next() {
        let is_last_empty = line.is_empty() && lines.peek().is_none();
        if is_last_empty {
            break;
        }

        if line.starts_with(b"diff --git ") {
            if let Some(f) = cur.take() {
                files.push(f);
            }
            cur = Some(FileDiff {
                headers: vec![line.to_vec()],
                old_path: None,
                new_path: None,
                content: FileContent::Text(Vec::new()),
            });
            in_hunk = false;
            continue;
        }

        // Plain (non-git) diff: a file starts at "--- " when not already building one,
        // or when the current file already has hunks (next file).
        if line.starts_with(b"--- ") && (cur.is_none() || in_hunk) {
            if let Some(f) = cur.take() {
                files.push(f);
            }
            cur = Some(FileDiff {
                headers: Vec::new(),
                old_path: None,
                new_path: None,
                content: FileContent::Text(Vec::new()),
            });
            in_hunk = false;
        }

        let Some(f) = cur.as_mut() else {
            continue; // preamble before any file
        };

        if line.starts_with(b"@@ ") {
            let hunk = parse_hunk_header(line)?;
            let FileContent::Text(hunks) = &mut f.content else {
                return Err(ParseError::Unexpected("hunk in binary file".into()));
            };
            hunks.push(hunk);
            in_hunk = true;
            continue;
        }

        if in_hunk {
            let FileContent::Text(hunks) = &mut f.content else {
                unreachable!()
            };
            let h = hunks.last_mut().unwrap();
            match line.first() {
                Some(b' ') => h.lines.push(mk_line(LineKind::Context, &line[1..])),
                Some(b'+') => h.lines.push(mk_line(LineKind::Add, &line[1..])),
                Some(b'-') => h.lines.push(mk_line(LineKind::Del, &line[1..])),
                _ if line.starts_with(b"\\ ") => {
                    if let Some(last) = h.lines.last_mut() {
                        last.no_newline = true;
                    }
                }
                _ => {
                    in_hunk = false;
                    push_header(f, line);
                }
            }
            continue;
        }

        push_header(f, line);
    }

    if let Some(f) = cur.take() {
        files.push(f);
    }
    Ok(Patch { files })
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
        no_newline: false,
    }
}

fn push_header(f: &mut FileDiff, line: &[u8]) {
    if line.starts_with(b"Binary files ") || line == b"GIT binary patch" {
        match &mut f.content {
            FileContent::Binary(b) => b.push(line.to_vec()),
            FileContent::Text(h) if h.is_empty() => {
                f.content = FileContent::Binary(vec![line.to_vec()]);
            }
            FileContent::Text(_) => { /* binary marker never follows hunks */ }
        }
        return;
    }
    if let Some(rest) = line.strip_prefix(b"--- ") {
        f.old_path = Some(strip_ab(rest));
    } else if let Some(rest) = line.strip_prefix(b"+++ ") {
        f.new_path = Some(strip_ab(rest));
    }
    f.headers.push(line.to_vec());
}

/// Strip a leading `a/` or `b/`, drop a trailing tab-and-timestamp if present.
fn strip_ab(s: &[u8]) -> Vec<u8> {
    let s = match s.iter().position(|&b| b == b'\t') {
        Some(i) => &s[..i],
        None => s,
    };
    let s = s
        .strip_prefix(b"a/")
        .or_else(|| s.strip_prefix(b"b/"))
        .unwrap_or(s);
    s.to_vec()
}

fn parse_hunk_header(line: &[u8]) -> Result<Hunk, ParseError> {
    // Format: @@ -os[,ol] +ns[,nl] @@[ section]
    let bad = || ParseError::BadHunkHeader(String::from_utf8_lossy(line).into_owned());
    let body = line.strip_prefix(b"@@ ").ok_or_else(bad)?;
    let end = find_subslice(body, b" @@").ok_or_else(bad)?;
    let ranges = &body[..end];
    let after = &body[end + 3..];
    let section = after.strip_prefix(b" ").unwrap_or(after).to_vec();
    // The ranges portion (`-os,ol +ns,nl`) is ASCII for any valid hunk header.
    let ranges = std::str::from_utf8(ranges).map_err(|_| bad())?;
    let mut it = ranges.split_whitespace();
    let old = it.next().ok_or_else(bad)?;
    let new = it.next().ok_or_else(bad)?;
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
    Ok((start, count))
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(h[0].lines[0].no_newline);
        assert!(h[0].lines[1].no_newline);
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
