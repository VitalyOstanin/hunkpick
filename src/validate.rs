use crate::model::*;
use crate::renumber::{anchor, expected_new_start};
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Which side of a hunk header a count belongs to.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Side {
    /// The `-` side: context and deleted lines.
    Old,
    /// The `+` side: context and added lines.
    New,
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Side::Old => "old",
            Side::New => "new",
        })
    }
}

/// A diff that does not add up. `hunk_index` is 0-based internally; [`fmt::Display`] renders it
/// 1-based, as `list` and the selectors number sub-hunks.
#[derive(Debug, PartialEq, Eq)]
pub enum ValidationError {
    /// A hunk header declares a different number of old- or new-side lines than its body holds.
    CountMismatch {
        /// Display path of the file the hunk belongs to.
        file: String,
        /// 0-based position of the hunk within that file.
        hunk_index: usize,
        /// Which side's count disagrees.
        side: Side,
        /// The value the `@@` header declares.
        header: u32,
        /// The value the body actually holds.
        body: u32,
    },
    /// A hunk starts before the end of the one before it in the same file, on either side.
    OverlappingHunks {
        /// Display path of the file the hunk belongs to.
        file: String,
        /// 0-based position of the hunk within that file.
        hunk_index: usize,
    },
    /// A hunk with no body lines at all: git rejects the `@@ -X,0 +Y,0 @@` stanza it emits.
    EmptyHunk {
        /// Display path of the file the hunk belongs to.
        file: String,
        /// 0-based position of the hunk within that file.
        hunk_index: usize,
    },
    /// A hunk whose body is all context (no `+`/`-` lines). The count checks pass it
    /// (`old_lines == new_lines == ctx`), yet `git apply` rejects a hunk with no changes.
    /// Unreachable from a real git diff (git never emits a change-free hunk) but possible
    /// from a synthetic patch, so reject it explicitly.
    NoChangeHunk {
        /// Display path of the file the hunk belongs to.
        file: String,
        /// 0-based position of the hunk within that file.
        hunk_index: usize,
    },
    /// A hunk whose new-side start does not follow from the old-side start plus the net size
    /// of the hunks before it in the same file. Carries the header's value and the one the
    /// diff implies. This is what an anchor carried over from a larger input diff looks like:
    /// the counts and the body are consistent, yet `git apply` — which searches from the
    /// new-side position — starts at the wrong line (see [`crate::renumber`]).
    StaleNewStart {
        /// Display path of the file the hunk belongs to.
        file: String,
        /// 0-based position of the hunk within that file.
        hunk_index: usize,
        /// The new-side start the `@@` header declares.
        header: u32,
        /// The new-side start the diff itself implies.
        expected: u32,
    },
}

impl fmt::Display for ValidationError {
    /// Phrased for the user: the file, the sub-hunk numbered from one as `list` and the
    /// selectors number it, and what did not add up. The variants' field names are internal.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationError::CountMismatch {
                file,
                hunk_index,
                side,
                header,
                body,
            } => {
                write!(
                    f,
                    "{file}: sub-hunk {}: header declares {header} {side} lines, body has {body}",
                    hunk_index + 1
                )
            }
            ValidationError::OverlappingHunks { file, hunk_index } => write!(
                f,
                "{file}: sub-hunk {} overlaps the one before it",
                hunk_index + 1
            ),
            ValidationError::EmptyHunk { file, hunk_index } => {
                write!(f, "{file}: sub-hunk {} has an empty body", hunk_index + 1)
            }
            ValidationError::NoChangeHunk { file, hunk_index } => write!(
                f,
                "{file}: sub-hunk {} has no added or deleted lines",
                hunk_index + 1
            ),
            ValidationError::StaleNewStart {
                file,
                hunk_index,
                header,
                expected,
            } => write!(
                f,
                "{file}: sub-hunk {}: header starts the new side at line {header}, \
                 but the diff puts it at {expected}",
                hunk_index + 1
            ),
        }
    }
}

/// Lets callers treat it as a boxed [`std::error::Error`], as the Rust API guidelines ask
/// of a public error type.
impl std::error::Error for ValidationError {}

/// Consistency check of an INPUT diff: everything that hunkpick will not fix on its way out.
/// New-side anchors are deliberately not checked — `select` recomputes them (see
/// [`crate::renumber`]), so a diff carrying anchors from a larger diff is valid input.
pub fn validate_input(patch: &Patch) -> Result<(), ValidationError> {
    check_hunks(patch, false)
}

/// Internal consistency check of a result diff. Git-agnostic, O(total lines).
pub fn validate_internal(patch: &Patch) -> Result<(), ValidationError> {
    check_hunks(patch, true)
}

fn check_hunks(patch: &Patch, check_new_start: bool) -> Result<(), ValidationError> {
    for f in &patch.files {
        let path = f.display_path();
        let FileContent::Text(hunks) = &f.content else {
            continue; // binary files have no hunk bodies to check
        };
        // Positions are normalised through `renumber::anchor` and computed in i64. Both reasons
        // matter: a side with a zero count reports the line *before* its empty range (git's
        // convention), so raw header numbers make a pure deletion following another hunk look
        // like an overlap; and a sum near u32::MAX would panic in a debug build and wrap in a
        // release one, deciding the comparison on a meaningless number.
        let mut prev_old_end: Option<i64> = None;
        let mut prev_new_end: Option<i64> = None;
        // Net `added - deleted` of the hunks already seen in this file: what the new-side
        // anchor of every later hunk is shifted by.
        let mut delta: i64 = 0;
        for (i, h) in hunks.iter().enumerate() {
            let (add, del) = check_one_hunk(h, &path, i, check_new_start, delta)?;
            delta += i64::from(add) - i64::from(del);
            let old_at = anchor(h.old_start, h.old_lines);
            let new_at = anchor(h.new_start, h.new_lines);
            if prev_old_end.is_some_and(|pe| old_at < pe)
                || prev_new_end.is_some_and(|pe| new_at < pe)
            {
                return Err(ValidationError::OverlappingHunks {
                    file: path.clone(),
                    hunk_index: i,
                });
            }
            prev_old_end = Some(old_at + i64::from(h.old_lines));
            prev_new_end = Some(new_at + i64::from(h.new_lines));
        }
    }
    Ok(())
}

/// Everything checkable about one hunk on its own: a non-empty body that carries a change, header
/// counts that match that body, and (for a result diff) a new-side anchor that follows from
/// `delta`, the net `added - deleted` of the hunks before it in the same file. Returns the hunk's
/// own added/deleted counts so the caller can advance `delta`.
fn check_one_hunk(
    h: &Hunk,
    path: &str,
    index: usize,
    check_new_start: bool,
    delta: i64,
) -> Result<(u32, u32), ValidationError> {
    // A text hunk with no body lines emits a `@@ -X,0 +Y,0 @@` stanza git rejects
    // as a corrupt patch. The count checks below pass it (0 == 0), so reject it here.
    if h.lines.is_empty() {
        return Err(ValidationError::EmptyHunk {
            file: path.to_string(),
            hunk_index: index,
        });
    }
    let (ctx, add, del) = count_kinds(&h.lines);
    // A change-free (all-context) hunk passes the count checks but git apply rejects it.
    // `EmptyHunk` above only catches a zero-line body, so guard the non-empty all-context
    // case here.
    if add == 0 && del == 0 {
        return Err(ValidationError::NoChangeHunk {
            file: path.to_string(),
            hunk_index: index,
        });
    }
    if h.old_lines != ctx + del {
        return Err(ValidationError::CountMismatch {
            file: path.to_string(),
            hunk_index: index,
            side: Side::Old,
            header: h.old_lines,
            body: ctx + del,
        });
    }
    if h.new_lines != ctx + add {
        return Err(ValidationError::CountMismatch {
            file: path.to_string(),
            hunk_index: index,
            side: Side::New,
            header: h.new_lines,
            body: ctx + add,
        });
    }
    // The new-side start is not an independent value: it follows from the old-side start and
    // everything this diff already changed above. A hunk carried over from a larger diff keeps
    // the anchor of that diff and passes every check above, so check it explicitly rather than
    // leaving it for `git apply` to mis-locate.
    let expected = expected_new_start(h, delta);
    if check_new_start && h.new_start != expected {
        return Err(ValidationError::StaleNewStart {
            file: path.to_string(),
            hunk_index: index,
            header: h.new_start,
            expected,
        });
    }
    Ok((add, del))
}

/// Why a `git apply --check` run did not end in a verdict of "applies".
///
/// The distinction the caller needs is between *the result diff is bad* and *the check could not
/// be made*: only the first says anything about hunkpick's output. Merging them into one string
/// reported a missing `git` as a rejected diff.
#[derive(Debug)]
pub enum GitCheckError {
    /// `git` could not be started — absent from `PATH`, not executable, no fork available, or
    /// the working directory it was to run in unusable. Carries that directory: `spawn` reports
    /// a missing binary and a missing directory with the same `NotFound`, and the message has to
    /// tell the caller which of the two they are looking at.
    Spawn {
        /// What `Command::spawn` reported.
        source: std::io::Error,
        /// The working directory the child was to run in (the value of `-C DIR`).
        dir: PathBuf,
    },
    /// Feeding the diff to git, or collecting its output, failed.
    Io(std::io::Error),
    /// The thread writing the diff to git's stdin panicked.
    WriterPanicked,
    /// git ran and refused the diff. Carries its stderr.
    Rejected(String),
    /// git ran but never reached a verdict: a fatal error of its own (a broken repository, an
    /// unusable configuration — exit 128) or death by signal (`code` is `None` on Unix). This
    /// says nothing about the diff, so it must not be reported as a rejection.
    Failed {
        /// git's exit status, or `None` when a signal ended it.
        code: Option<i32>,
        /// Whatever git managed to say; empty when it was killed outright.
        stderr: String,
    },
}

impl fmt::Display for GitCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitCheckError::Spawn { source, dir } => {
                write!(f, "failed to run git in {}: {source}", dir.display())
            }
            GitCheckError::Io(e) => write!(f, "git check failed: {e}"),
            GitCheckError::WriterPanicked => write!(f, "the thread feeding git panicked"),
            GitCheckError::Rejected(stderr) => {
                write!(f, "git apply --check rejected the result diff: {stderr}")
            }
            GitCheckError::Failed { code, stderr } => {
                let how = match code {
                    Some(c) => format!("exited with code {c}"),
                    None => "was killed by a signal".to_string(),
                };
                // An empty stderr is the whole point of naming the status: a git killed by the
                // OOM killer says nothing at all, and "no verdict" with a blank reason reads as
                // a verdict with a blank reason.
                if stderr.is_empty() {
                    write!(
                        f,
                        "git apply --check {how} without a diagnostic; the result diff was not checked"
                    )
                } else {
                    write!(
                        f,
                        "git apply --check {how} without checking the result diff: {stderr}"
                    )
                }
            }
        }
    }
}

impl std::error::Error for GitCheckError {}

/// Run `git apply --check` against the working tree in `dir`, feeding `diff_bytes` on stdin.
/// Returns Err with git's verdict, or with the reason the check could not be made.
pub fn validate_with_git(diff_bytes: &[u8], dir: &Path) -> Result<(), GitCheckError> {
    let mut cmd = Command::new("git");
    cmd.arg("apply")
        .arg("--check")
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // `git apply --check` without `--index`/`--cached` reads the working tree of the current
    // directory, so these variables do not affect this call as it stands (verified against
    // git 2.53.0). They do decide the repository once the index is involved, and they arrive
    // set from hooks, `git rebase --exec` and editor integrations — dropping them keeps
    // `-C DIR` the only thing that selects the repository whatever flags are added later.
    crate::gitenv::insulate_repo_location(&mut cmd);
    // git's stderr below is decoded lossily and shown next to hunkpick's own ASCII-English
    // text; pinning the locale keeps it English and keeps its bytes UTF-8.
    crate::gitenv::pin_message_locale(&mut cmd);
    let mut child = cmd.spawn().map_err(|source| GitCheckError::Spawn {
        source,
        dir: dir.to_path_buf(),
    })?;
    // Feed stdin from a separate thread while this one drains stdout/stderr. Writing the whole
    // diff first would deadlock if git filled its stderr pipe (typically 64 KiB) before reading
    // the patch to the end — plausible on a large diff that git rejects hunk by hunk.
    let mut stdin = child.stdin.take().expect("stdin was configured as piped");
    let writer = std::thread::scope(|scope| {
        let handle = scope.spawn(move || stdin.write_all(diff_bytes));
        let output = child.wait_with_output();
        // Both sides are joined here: the writer cannot outlive the scope, so nothing is left
        // running once this returns.
        (handle.join(), output)
    });
    let (write_result, output) = writer;
    match write_result {
        // A closed pipe means git stopped reading (it rejected the patch early); its exit
        // status and stderr below carry the real diagnosis, so this is not the error to report.
        Ok(Err(e)) if e.kind() != std::io::ErrorKind::BrokenPipe => {
            return Err(GitCheckError::Io(e));
        }
        Err(_) => return Err(GitCheckError::WriterPanicked),
        _ => {}
    }
    let output = output.map_err(GitCheckError::Io)?;
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    match output.status.code() {
        Some(0) => Ok(()),
        // `git apply --check` answers 1, and only 1, when the patch does not apply; that is the
        // only status carrying a verdict about the diff. 128 is a fatal error of git's own and
        // `None` is death by signal — neither looked at the patch, and calling either a
        // rejection blames hunkpick's output for a broken environment (ADR 0013 keeps exit 70
        // for a result hunkpick itself produced).
        Some(1) => Err(GitCheckError::Rejected(stderr)),
        code => Err(GitCheckError::Failed { code, stderr }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gittest::repo_with_file;
    use crate::parser::parse;

    /// A one-change diff that is internally consistent: the fixture the checks pass on, and
    /// the base several tests mutate to make one of them fail.
    const ONE_CHANGE: &str = "\
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 a
-b
+B
 c
";

    #[test]
    fn well_formed_diff_passes() {
        let p = parse(ONE_CHANGE.as_bytes()).unwrap();
        assert!(validate_internal(&p).is_ok());
    }

    #[test]
    fn stale_new_start_is_caught() {
        // The header of a hunk taken out of a larger diff: counts and body agree, but the
        // new-side start still describes the file the full diff produced.
        let p = parse(
            "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -17,3 +18,3 @@
 q
-r
+R
 s
"
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(
            validate_internal(&p),
            Err(ValidationError::StaleNewStart {
                file: "f".to_string(),
                hunk_index: 0,
                header: 18,
                expected: 17,
            })
        );
    }

    #[test]
    fn accumulated_offset_across_hunks_passes() {
        // Two hunks of one diff: the first removes a line net, so the second starts one line
        // earlier on the new side. The check must accept exactly that.
        let p = parse(
            "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -2,3 +2,2 @@
 b
-c
 d
@@ -17,3 +16,3 @@
 q
-r
+R
 s
"
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(validate_internal(&p), Ok(()));
    }

    /// A hunk with no new-side lines reports the line *before* its empty range, so its header
    /// number equals the last line of the hunk above it. Comparing the raw numbers reads that as
    /// an overlap; the positions have to be normalised the way `renumber::anchor` does.
    #[test]
    fn a_pure_deletion_after_another_hunk_is_not_an_overlap() {
        // What `git diff` gives for a,b,c,d,e -> a,c, split at the context gap: the second
        // hunk's `+2,0` is exactly what `git diff -U0` writes for the same change.
        let p = parse(
            "\
--- a/f
+++ b/f
@@ -1,3 +1,2 @@
 a
-b
 c
@@ -4,2 +2,0 @@
-d
-e
"
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(validate_internal(&p), Ok(()));
        assert_eq!(validate_input(&p), Ok(()));
    }

    /// The mirror case on the old side: a pure insertion carries the line before its empty
    /// old-side range, which is the last old line the hunk above covers.
    #[test]
    fn a_pure_insertion_after_another_hunk_is_not_an_overlap() {
        let p = parse(
            "\
--- a/f
+++ b/f
@@ -1,2 +1,2 @@
-a
+A
 b
@@ -2,0 +3,2 @@
+x
+y
"
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(validate_internal(&p), Ok(()));
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
    fn all_context_hunk_is_caught() {
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
        // Turn the change lines into context: a change-free hunk whose counts still balance
        // (old_lines == new_lines == ctx) but which git apply rejects.
        if let FileContent::Text(h) = &mut p.files[0].content {
            for l in &mut h[0].lines {
                l.kind = LineKind::Context;
            }
            h[0].old_lines = 3;
            h[0].new_lines = 3;
        }
        assert!(matches!(
            validate_internal(&p),
            Err(ValidationError::NoChangeHunk { .. })
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
        let dir = repo_with_file("a\nb\nc\n");
        assert!(validate_with_git(ONE_CHANGE.as_bytes(), dir.path()).is_ok());
    }

    #[test]
    fn git_check_rejects_bad_result() {
        let dir = repo_with_file("totally\ndifferent\ncontent\n");
        assert!(validate_with_git(ONE_CHANGE.as_bytes(), dir.path()).is_err());
    }
}
