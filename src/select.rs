use crate::model::*;
use crate::split::auto_split_hunk;

#[derive(Debug, PartialEq, Eq)]
pub enum Selector {
    File {
        path: Option<String>,
        indices: Vec<usize>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum SelectError {
    BadSelector(String),
    UnknownPath(String),
    AmbiguousPath(String),
    NoIndex(String),
    EmptySelection,
}

/// Per-file auto-split view: each file maps to its ordered sub-hunks (empty for binary).
pub fn build_view(patch: &Patch) -> Vec<(usize, Vec<Hunk>)> {
    let mut view = Vec::new();
    for (fi, f) in patch.files.iter().enumerate() {
        match &f.content {
            FileContent::Text(hunks) => {
                let mut subs = Vec::new();
                for h in hunks {
                    subs.extend(auto_split_hunk(h));
                }
                view.push((fi, subs));
            }
            FileContent::Binary(_) => view.push((fi, Vec::new())),
        }
    }
    view
}

/// Parse selector args. A selector is `path:list` or (for single-file diffs) just `list`,
/// where `list` is comma-separated indices and ranges, e.g. `1,3` or `2-4` or `src/f:1,3-5`.
/// A `path:list` form is recognised when a ':' is present and the text after the LAST ':'
/// parses as a valid index list.
pub fn parse_selectors(args: &[String]) -> Result<Vec<Selector>, SelectError> {
    let mut out = Vec::new();
    for a in args {
        let (path, list) = match a.rsplit_once(':') {
            Some((p, l)) if !p.is_empty() && parse_index_list(l).is_ok() => {
                (Some(p.to_string()), l.to_string())
            }
            _ => (None, a.clone()),
        };
        let indices = parse_index_list(&list).map_err(|_| SelectError::BadSelector(a.clone()))?;
        out.push(Selector::File { path, indices });
    }
    Ok(out)
}

fn parse_index_list(s: &str) -> Result<Vec<usize>, ()> {
    if s.is_empty() {
        return Err(());
    }
    let mut v = Vec::new();
    for part in s.split(',') {
        if let Some((lo, hi)) = part.split_once('-') {
            let lo: usize = lo.parse().map_err(|_| ())?;
            let hi: usize = hi.parse().map_err(|_| ())?;
            if lo == 0 || hi < lo {
                return Err(());
            }
            v.extend(lo..=hi);
        } else {
            let n: usize = part.parse().map_err(|_| ())?;
            if n == 0 {
                return Err(());
            }
            v.push(n);
        }
    }
    Ok(v)
}

pub fn select(patch: &Patch, selectors: &[Selector]) -> Result<Patch, SelectError> {
    use std::collections::BTreeMap;
    let view = build_view(patch);
    let mut chosen: BTreeMap<usize, Vec<usize>> = BTreeMap::new();

    for sel in selectors {
        let Selector::File { path, indices } = sel;
        let fi = resolve_file(patch, path.as_deref())?;
        let subs = &view.iter().find(|(f, _)| *f == fi).unwrap().1;
        let is_binary = matches!(patch.files[fi].content, FileContent::Binary(_));
        for &idx in indices {
            if !is_binary && idx > subs.len() {
                let pname = path
                    .clone()
                    .unwrap_or_else(|| patch.files[fi].display_path());
                return Err(SelectError::NoIndex(format!("{pname}:{idx}")));
            }
            chosen.entry(fi).or_default().push(idx);
        }
    }
    if chosen.is_empty() {
        return Err(SelectError::EmptySelection);
    }

    let mut files = Vec::new();
    for (fi, mut idxs) in chosen {
        idxs.sort_unstable();
        idxs.dedup();
        let src = &patch.files[fi];
        let content = match &src.content {
            FileContent::Binary(b) => FileContent::Binary(b.clone()),
            FileContent::Text(_) => {
                let subs = &view.iter().find(|(f, _)| *f == fi).unwrap().1;
                FileContent::Text(idxs.iter().map(|&i| subs[i - 1].clone()).collect())
            }
        };
        files.push(FileDiff {
            headers: src.headers.clone(),
            old_path: src.old_path.clone(),
            new_path: src.new_path.clone(),
            content,
        });
    }
    Ok(Patch { files })
}

/// Resolve an optional path to a file index. With no path, succeeds only for single-file diffs.
pub(crate) fn resolve_file(patch: &Patch, path: Option<&str>) -> Result<usize, SelectError> {
    match path {
        None => {
            if patch.files.len() == 1 {
                Ok(0)
            } else {
                Err(SelectError::AmbiguousPath(
                    "<no path on multi-file diff>".into(),
                ))
            }
        }
        Some(p) => {
            let matches: Vec<usize> = patch
                .files
                .iter()
                .enumerate()
                .filter(|(_, f)| {
                    f.new_path.as_deref() == Some(p.as_bytes())
                        || f.old_path.as_deref() == Some(p.as_bytes())
                })
                .map(|(i, _)| i)
                .collect();
            match matches.as_slice() {
                [one] => Ok(*one),
                [] => Err(SelectError::UnknownPath(p.to_string())),
                _ => Err(SelectError::AmbiguousPath(p.to_string())),
            }
        }
    }
}

/// Resolve `path:N` / `N` to (file_index, original_hunk_index_0based) for the `split` command.
pub fn resolve_hunk(patch: &Patch, addr: &str) -> Result<(usize, usize), SelectError> {
    let (path, nstr) = match addr.rsplit_once(':') {
        Some((p, n)) if !p.is_empty() && n.parse::<usize>().is_ok() => (Some(p.to_string()), n),
        _ => (None, addr),
    };
    let n: usize = nstr
        .parse()
        .map_err(|_| SelectError::BadSelector(addr.to_string()))?;
    if n == 0 {
        return Err(SelectError::BadSelector(addr.to_string()));
    }
    let fi = resolve_file(patch, path.as_deref())?;
    match &patch.files[fi].content {
        FileContent::Text(h) if n <= h.len() => Ok((fi, n - 1)),
        FileContent::Text(_) => Err(SelectError::NoIndex(addr.to_string())),
        FileContent::Binary(_) => Err(SelectError::BadSelector(format!("{addr} (binary file)"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emit::emit;
    use crate::parser::parse;

    const MULTI: &str = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,5 +1,5 @@
 a
-b
+B
 c
-d
+D
 e
";

    #[test]
    fn parse_selector_bare_index_list() {
        let sels = parse_selectors(&["1,2".to_string()]).unwrap();
        assert_eq!(sels.len(), 1);
        let Selector::File { path, indices } = &sels[0];
        assert!(path.is_none());
        assert_eq!(indices, &vec![1, 2]);
    }

    #[test]
    fn parse_selector_path_with_range() {
        let sels = parse_selectors(&["src/f:2-4".to_string()]).unwrap();
        let Selector::File { path, indices } = &sels[0];
        assert_eq!(path.as_deref(), Some("src/f"));
        assert_eq!(indices, &vec![2, 3, 4]);
    }

    #[test]
    fn select_first_subhunk_only() {
        let p = parse(MULTI.as_bytes()).unwrap();
        let sels = parse_selectors(&["1".to_string()]).unwrap();
        let out = select(&p, &sels).unwrap();
        let text = String::from_utf8(emit(&out)).unwrap();
        assert!(text.contains("+B"));
        assert!(!text.contains("+D")); // second change excluded
    }

    #[test]
    fn select_unknown_index_errors() {
        let p = parse(MULTI.as_bytes()).unwrap();
        let sels = parse_selectors(&["9".to_string()]).unwrap();
        assert!(matches!(select(&p, &sels), Err(SelectError::NoIndex(_))));
    }

    #[test]
    fn select_empty_is_error() {
        let p = parse(MULTI.as_bytes()).unwrap();
        assert_eq!(select(&p, &[]), Err(SelectError::EmptySelection));
    }

    #[test]
    fn resolve_hunk_addresses_original_hunk() {
        let p = parse(MULTI.as_bytes()).unwrap();
        assert_eq!(resolve_hunk(&p, "1").unwrap(), (0, 0));
    }

    #[test]
    fn select_second_subhunk_applies_via_git() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let p = parse(MULTI.as_bytes()).unwrap();
        // Select only the SECOND sub-hunk (the d->D change).
        let sels = parse_selectors(&["2".to_string()]).unwrap();
        let out = select(&p, &sels).unwrap();
        let diff = emit(&out);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), "a\nb\nc\nd\ne\n").unwrap();
        Command::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(&dir)
            .status()
            .unwrap();
        let mut child = Command::new("git")
            .arg("apply")
            .arg("--check")
            .current_dir(&dir)
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&diff).unwrap();
        assert!(
            child.wait().unwrap().success(),
            "second-only sub-hunk failed to apply:\n{}",
            String::from_utf8_lossy(&diff)
        );
    }
}
