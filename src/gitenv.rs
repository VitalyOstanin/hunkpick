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

#[cfg(test)]
mod tests {
    use super::*;

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
