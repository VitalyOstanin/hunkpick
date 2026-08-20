//! The environment variables that decide which repository a `git` invocation acts on.
//!
//! Every place that runs `git` — the result-diff check in [`crate::validate`], the unit-test
//! helpers, the integration tests — has to drop the same set, and a list copied per call site
//! drifts: three copies of it had already grown three different memberships. One list, one
//! meaning.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Output};

/// Variables through which the surrounding process points git at a repository, an index or an
/// object store. They arrive set from hooks, `git rebase --exec` and editor integrations, so a
/// `git` child inherits them unless they are dropped, and then `-C DIR` (or `current_dir`) is
/// no longer what selects the repository.
pub const REPO_LOCATING_VARS: [&str; 7] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CEILING_DIRECTORIES",
];

/// Drop [`REPO_LOCATING_VARS`] from `cmd`, so the directory it runs in is the only thing that
/// decides which repository it acts on.
pub fn insulate_repo_location(cmd: &mut Command) {
    for var in REPO_LOCATING_VARS {
        cmd.env_remove(var);
    }
}

/// Variables through which the surrounding process selects the language and character encoding
/// of git's own messages. `git apply --check` writes its diagnosis to stderr, hunkpick decodes
/// that with `String::from_utf8_lossy` and puts it in front of the user next to its own
/// ASCII-English text, so an inherited locale would show up there as another language, or as
/// U+FFFD where the bytes are not UTF-8.
pub const MESSAGE_LOCALE_VARS: [&str; 4] = ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"];
/// The locale asked of a `git` child: the C locale, whose messages are English and ASCII.
pub const C_LOCALE: &str = "C";

/// Pin the message locale of `cmd` to [`C_LOCALE`], so what git says about a rejected diff is
/// English ASCII whatever the surrounding locale is. `LANGUAGE` is dropped rather than set:
/// gettext reads it as a list of languages to try and ignores it only while the locale is `C`,
/// which leaves nothing to gain by keeping it.
pub fn pin_message_locale(cmd: &mut Command) {
    cmd.env("LC_ALL", C_LOCALE);
    cmd.env("LC_MESSAGES", C_LOCALE);
    cmd.env("LANG", C_LOCALE);
    cmd.env_remove("LANGUAGE");
}

/// The file name given to git as its global configuration when a caller wants none.
pub const ABSENT_GLOBAL_CONFIG: &str = "absent-global-gitconfig";
/// The file name given to git as its system configuration when a caller wants none.
pub const ABSENT_SYSTEM_CONFIG: &str = "absent-system-gitconfig";

/// Point git's global and system configuration at files that do not exist inside `scratch_dir`,
/// so a developer's own settings cannot change what the child sees. Git reads a missing
/// configuration file as an empty one, which is the point: there is no "ignore my config" flag.
///
/// This is for throwaway repositories built by tests. The result-diff check in
/// [`crate::validate`] deliberately does not do it — that run happens in the caller's own tree,
/// where their `apply.whitespace` and friends are part of the answer they asked for.
pub fn insulate_config(cmd: &mut Command, scratch_dir: &Path) {
    cmd.env("GIT_CONFIG_GLOBAL", scratch_dir.join(ABSENT_GLOBAL_CONFIG));
    cmd.env("GIT_CONFIG_SYSTEM", scratch_dir.join(ABSENT_SYSTEM_CONFIG));
}

/// Why feeding a child process its input did not produce an [`Output`].
///
/// The distinction that matters to a caller is between "the child answered" and "we never got an
/// answer": only the first says anything about what was fed in.
#[derive(Debug)]
pub enum FeedError {
    /// The process could not be started.
    Spawn(std::io::Error),
    /// Writing the input failed for a reason other than the child closing its end.
    Write(std::io::Error),
    /// The thread writing the input panicked.
    WriterPanicked,
    /// Reading the child's output, or waiting for it, failed.
    Wait(std::io::Error),
}

impl std::fmt::Display for FeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeedError::Spawn(e) => write!(f, "could not start the process: {e}"),
            FeedError::Write(e) => write!(f, "could not write the input: {e}"),
            FeedError::WriterPanicked => write!(f, "the thread feeding the input panicked"),
            FeedError::Wait(e) => write!(f, "could not collect the output: {e}"),
        }
    }
}

impl std::error::Error for FeedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FeedError::Spawn(e) | FeedError::Write(e) | FeedError::Wait(e) => Some(e),
            FeedError::WriterPanicked => None,
        }
    }
}

/// Run `cmd` with `bytes` on its stdin and collect its output.
///
/// The input is written from a separate thread while this one drains the pipes. Writing the
/// whole input first would deadlock as soon as the child filled a pipe of its own (typically
/// 64 KiB) before reading the input to the end — which is exactly what `git apply --check` does
/// on a large diff it rejects hunk by hunk, reporting one line per hunk while the diff is still
/// arriving.
///
/// The child is kept in hand rather than consumed by `wait_with_output`, so that a failure of
/// the wait can `kill` it: the writing thread may be blocked in `write` at that moment, and
/// [`std::thread::scope`] parks this thread until every thread it started has finished before it
/// propagates anything. Without the kill, a failure there would hang instead of being reported.
///
/// `cmd`'s stdio is configured here; whatever the caller set is replaced.
pub fn feed_and_wait(cmd: &mut Command, bytes: &[u8]) -> Result<Output, FeedError> {
    use std::process::Stdio;
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(FeedError::Spawn)?;
    let mut stdin = child.stdin.take().expect("stdin was configured as piped");
    let mut child_stdout = child.stdout.take().expect("stdout was configured as piped");
    let mut child_stderr = child.stderr.take().expect("stderr was configured as piped");

    let (written, stdout, stderr, status) = std::thread::scope(|scope| {
        let writer = scope.spawn(move || stdin.write_all(bytes));
        let out = scope.spawn(move || read_to_end(&mut child_stdout));
        let err = scope.spawn(move || read_to_end(&mut child_stderr));
        let status = wait_or_kill(&mut child);
        (writer.join(), out.join(), err.join(), status)
    });

    match written {
        // A closed pipe means the child stopped reading — it reached its answer early. Its exit
        // status and output carry the diagnosis, so this is not the error to report.
        Ok(Err(e)) if e.kind() != std::io::ErrorKind::BrokenPipe => {
            return Err(FeedError::Write(e));
        }
        Err(_) => return Err(FeedError::WriterPanicked),
        _ => {}
    }
    let stdout = stdout.map_err(|_| FeedError::WriterPanicked)?;
    let stderr = stderr.map_err(|_| FeedError::WriterPanicked)?;
    Ok(Output {
        status: status.map_err(FeedError::Wait)?,
        stdout: stdout.map_err(FeedError::Wait)?,
        stderr: stderr.map_err(FeedError::Wait)?,
    })
}

/// Wait for `child`, killing it if the wait itself fails. The kill is what lets a writing thread
/// blocked on the child's stdin come back (through `EPIPE`) instead of holding the scope open.
fn wait_or_kill(child: &mut Child) -> std::io::Result<std::process::ExitStatus> {
    let status = child.wait();
    if status.is_err() {
        let _ = child.kill();
    }
    status
}

fn read_to_end(pipe: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    pipe.read_to_end(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both configuration files git can read are pointed at names that do not exist. Dropping
    /// one of the two leaves a machine-dependent setting reaching the test repositories, which
    /// is what this whole module exists to prevent.
    #[test]
    fn config_insulation_covers_both_files_git_reads() {
        let mut cmd = Command::new("git");
        insulate_config(&mut cmd, Path::new("/scratch"));
        let envs: Vec<_> = cmd.get_envs().collect();
        for (var, name) in [
            ("GIT_CONFIG_GLOBAL", ABSENT_GLOBAL_CONFIG),
            ("GIT_CONFIG_SYSTEM", ABSENT_SYSTEM_CONFIG),
        ] {
            let (_, value) = envs
                .iter()
                .find(|(k, _)| *k == var)
                .unwrap_or_else(|| panic!("{var} must be set"));
            assert_eq!(
                value.map(Path::new),
                Some(Path::new("/scratch").join(name).as_path()),
                "{var} must point inside the scratch directory"
            );
        }
    }

    /// Input and output both larger than a pipe buffer come back whole. The child is fed from
    /// one thread while both of its pipes are drained from others; a version that wrote the
    /// input first and read afterwards would be at the mercy of whether the child happens to
    /// buffer the whole input before answering.
    #[test]
    fn a_child_that_answers_at_length_is_read_to_the_end() {
        let dir = crate::gittest::repo_with_file("a\n");
        // 4000 files that are not in the tree: git rejects each one and says so, which is about
        // 150 KB of stderr against 200 KB of input.
        let mut diff = String::new();
        for i in 0..4000 {
            diff.push_str(&format!(
                "--- a/f{i}\n+++ b/f{i}\n@@ -1 +1 @@\n-a{i}\n+b{i}\n"
            ));
        }
        let mut cmd = Command::new("git");
        cmd.arg("apply").arg("--check").current_dir(dir.path());
        insulate_config(&mut cmd, dir.path());
        insulate_repo_location(&mut cmd);
        pin_message_locale(&mut cmd);

        let out = feed_and_wait(&mut cmd, diff.as_bytes()).expect("git ran and answered");
        assert!(!out.status.success(), "the diff does not apply");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.len() > 64 * 1024,
            "the child's answer must outgrow a pipe buffer for this to test anything: {} bytes",
            stderr.len()
        );
        assert!(
            stderr.contains("f3999"),
            "the tail of the answer must survive: {}",
            &stderr[stderr.len().saturating_sub(200)..]
        );
    }

    /// Every variable the module names is accounted for: the repository ones dropped, the
    /// locale ones pinned. A variable added to a list without a matching call site would
    /// otherwise sit there doing nothing.
    #[test]
    fn insulation_covers_every_variable_the_module_names() {
        let mut cmd = Command::new("git");
        insulate_repo_location(&mut cmd);
        pin_message_locale(&mut cmd);
        let envs: Vec<_> = cmd.get_envs().collect();
        for var in REPO_LOCATING_VARS {
            assert!(
                envs.iter().any(|(k, v)| *k == var && v.is_none()),
                "{var} must be dropped"
            );
        }
        for var in MESSAGE_LOCALE_VARS {
            let entry = envs.iter().find(|(k, _)| *k == var);
            let (_, value) = entry.unwrap_or_else(|| panic!("{var} must be set or dropped"));
            assert!(
                *value == Some(C_LOCALE.as_ref()) || (var == "LANGUAGE" && value.is_none()),
                "{var} must be pinned to the C locale"
            );
        }
    }
}
