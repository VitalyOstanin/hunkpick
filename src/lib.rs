//! Non-interactive `git add -p` alternative: pick and split unified-diff hunks by
//! index, range, or content id.
//!
//! `hunkpick` reads a unified diff from stdin (or a file), splits each hunk into
//! minimal sub-hunks, and emits only the selected ones — suitable for piping into
//! `git apply --cached`. The crate exposes the building blocks used by the CLI:
//!
//! - [`parser`] — parse a unified diff into a [`model::Patch`];
//! - [`select`] — resolve selectors (index, range, `path:*`, content id) and emit
//!   the chosen sub-hunks;
//! - [`split`] — split an original hunk at context boundaries;
//! - [`renumber`] — recompute new-side line numbers of a result diff;
//! - [`list`] — enumerate addressable sub-hunks (human-readable and JSON);
//! - [`validate`] — check internal consistency of a result diff;
//! - [`subhunk_id`] — stable content ids for sub-hunks;
//! - [`emit`] — render a [`model::Patch`] back to a unified diff;
//! - [`error`] — application errors with process exit codes.
//!
//! ```
//! use hunkpick::{emit, model, parser, select};
//!
//! // A hunk with two changes separated by context auto-splits into two sub-hunks.
//! let diff = "\
//! --- a/f
//! +++ b/f
//! @@ -1,5 +1,5 @@
//!  a
//! -b
//! +B
//!  c
//! -d
//! +D
//!  e
//! ";
//! let patch: model::Patch = parser::parse(diff.as_bytes())?;
//!
//! // Take only the second one; the result is a diff of its own, with the new-side
//! // anchors recomputed so `git apply` can locate it.
//! let selectors = select::parse_selectors(&["2".to_string()])?;
//! let out = select::select(&patch, &selectors)?;
//!
//! let text = String::from_utf8(emit::emit(&out)).unwrap();
//! assert!(text.contains("+D"));
//! assert!(!text.contains("+B"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! See the `README` for the selector grammar and command reference.

#![warn(missing_docs)]

/// Command-line surface: the argument types clap derives the CLI from, and colour resolution.
pub mod cli;
/// Render a [`model::Patch`] back to a unified diff, byte-for-byte for git-canonical input.
pub mod emit;
/// Application errors and the process exit codes they map to.
pub mod error;
/// The environment variables that decide which repository a `git` child acts on.
pub mod gitenv;
#[cfg(test)]
mod gittest;
/// Enumerate the addressable sub-hunks of a patch, human-readable or as JSON.
pub mod list;
/// The data model of a parsed diff: [`model::Patch`], [`model::FileDiff`], [`model::Hunk`].
pub mod model;
/// Parse a unified diff (git or plain) into a [`model::Patch`].
pub mod parser;
/// Recompute the new-side line numbers of a result diff from the diff itself.
pub mod renumber;
/// Resolve selectors (index, range, `path:*`, `@id`, `@L`) and build the result patch.
pub mod select;
/// Split hunks: automatically at context gaps, explicitly at given lines, or down to a subset
/// of changed lines.
pub mod split;
/// Stable, context-free content ids for sub-hunks (the `@<id>` selector form).
pub mod subhunk_id;
/// Consistency checks for an input or result diff, plus the optional `git apply --check` gate.
pub mod validate;
