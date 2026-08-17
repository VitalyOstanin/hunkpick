//! The environment variables that decide which repository a `git` invocation acts on.
//!
//! Every place that runs `git` — the result-diff check in [`crate::validate`], the unit-test
//! helpers, the integration tests — has to drop the same set, and a list copied per call site
//! drifts: three copies of it had already grown three different memberships. One list, one
//! meaning.

use std::process::Command;

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
