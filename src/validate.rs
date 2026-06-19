use crate::model::*;
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub enum ValidationError {
    CountMismatch {
        file: String,
        hunk_index: usize,
        field: &'static str,
        header: u32,
        body: u32,
    },
    OverlappingHunks {
        file: String,
        hunk_index: usize,
    },
    EmptyHunk {
        file: String,
        hunk_index: usize,
    },
}

/// Internal consistency check of a result diff. Git-agnostic, O(total lines).
pub fn validate_internal(patch: &Patch) -> Result<(), ValidationError> {
    for f in &patch.files {
        let path = f.display_path();
        let FileContent::Text(hunks) = &f.content else {
            continue; // binary files have no hunk bodies to check
        };
        let mut prev_old_end: Option<u32> = None;
        let mut prev_new_end: Option<u32> = None;
        for (i, h) in hunks.iter().enumerate() {
            // A text hunk with no body lines emits a `@@ -X,0 +Y,0 @@` stanza git rejects
            // as a corrupt patch. The count checks below pass it (0 == 0), so reject it here.
            if h.lines.is_empty() {
                return Err(ValidationError::EmptyHunk {
                    file: path.clone(),
                    hunk_index: i,
                });
            }
            let mut ctx = 0u32;
            let mut add = 0u32;
            let mut del = 0u32;
            for l in &h.lines {
                match l.kind {
                    LineKind::Context => ctx += 1,
                    LineKind::Add => add += 1,
                    LineKind::Del => del += 1,
                }
            }
            if h.old_lines != ctx + del {
                return Err(ValidationError::CountMismatch {
                    file: path.clone(),
                    hunk_index: i,
                    field: "old_lines",
                    header: h.old_lines,
                    body: ctx + del,
                });
            }
            if h.new_lines != ctx + add {
                return Err(ValidationError::CountMismatch {
                    file: path.clone(),
                    hunk_index: i,
                    field: "new_lines",
                    header: h.new_lines,
                    body: ctx + add,
                });
            }
            if let Some(pe) = prev_old_end {
                if h.old_start < pe {
                    return Err(ValidationError::OverlappingHunks {
                        file: path.clone(),
                        hunk_index: i,
                    });
                }
            }
            if let Some(pe) = prev_new_end {
                if h.new_start < pe {
                    return Err(ValidationError::OverlappingHunks {
                        file: path.clone(),
                        hunk_index: i,
                    });
                }
            }
            prev_old_end = Some(h.old_start + h.old_lines);
            prev_new_end = Some(h.new_start + h.new_lines);
        }
    }
    Ok(())
}

/// Run `git apply --check` against the working tree in `dir`, feeding `diff_bytes` on stdin.
/// Returns Err with git's stderr on failure (or if git could not be run).
pub fn validate_with_git(diff_bytes: &[u8], dir: &Path) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("git")
        .arg("apply")
        .arg("--check")
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run git: {e}"))?;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(diff_bytes)
        .map_err(|e| format!("failed to write to git: {e}"))?;
    let output = child
        .wait_with_output()
        .map_err(|e| format!("git wait failed: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "git apply --check rejected the result diff: {}",
            stderr.trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn well_formed_diff_passes() {
        let p = parse(
            "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 a
-b
+B
 c
"
            .as_bytes(),
        )
        .unwrap();
        assert!(validate_internal(&p).is_ok());
    }

    #[test]
    fn count_mismatch_is_caught() {
        let mut p = parse(
            "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 a
-b
+B
 c
"
            .as_bytes(),
        )
        .unwrap();
        // Corrupt the header count.
        if let FileContent::Text(h) = &mut p.files[0].content {
            h[0].old_lines = 99;
        }
        assert!(matches!(
            validate_internal(&p),
            Err(ValidationError::CountMismatch { .. })
        ));
    }

    #[test]
    fn empty_hunk_body_is_caught() {
        let mut p = parse(
            "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 a
-b
+B
 c
"
            .as_bytes(),
        )
        .unwrap();
        // A text hunk with no body lines and zero counts passes the count checks
        // (0 == 0) yet emits a `@@ -X,0 +Y,0 @@` stanza git rejects. Catch it explicitly.
        if let FileContent::Text(h) = &mut p.files[0].content {
            h[0].lines.clear();
            h[0].old_lines = 0;
            h[0].new_lines = 0;
        }
        assert!(matches!(
            validate_internal(&p),
            Err(ValidationError::EmptyHunk { .. })
        ));
    }

    #[test]
    fn overlapping_hunks_are_caught() {
        let mut p = parse(
            "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,2 +1,2 @@
 a
-b
+B
@@ -10,2 +10,2 @@
 p
-q
+Q
"
            .as_bytes(),
        )
        .unwrap();
        // Force the second hunk to overlap the first on the old side.
        if let FileContent::Text(h) = &mut p.files[0].content {
            h[1].old_start = 1;
            h[1].new_start = 1;
        }
        assert!(matches!(
            validate_internal(&p),
            Err(ValidationError::OverlappingHunks { .. })
        ));
    }

    #[test]
    fn git_check_accepts_valid_result() {
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), "a\nb\nc\n").unwrap();
        Command::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(&dir)
            .status()
            .unwrap();
        let diff = "\
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 a
-b
+B
 c
";
        assert!(validate_with_git(diff.as_bytes(), dir.path()).is_ok());
    }

    #[test]
    fn git_check_rejects_bad_result() {
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), "totally\ndifferent\ncontent\n").unwrap();
        Command::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(&dir)
            .status()
            .unwrap();
        let diff = "\
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 a
-b
+B
 c
";
        assert!(validate_with_git(diff.as_bytes(), dir.path()).is_err());
    }
}
