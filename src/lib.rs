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
//! use hunkpick::{emit, model, parser, select, validate};
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
//! // Screen the input before selecting: parsing accepts a hunk header whose counts
//! // disagree with its body, and selection copies that header into its result rather
//! // than repairing it. The CLI does this in `load_and_parse`; a library caller has to.
//! validate::validate_input(&patch)?;
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
///
/// Not part of the library contract. The binary and the integration tests are separate crates,
/// so these types cannot be `pub(crate)`; hiding them keeps `cargo semver-checks` out of them,
/// because a new CLI flag is a new public field of an exhaustively constructible struct and
/// would otherwise force a minor version on a release that changes nothing for a library user.
#[doc(hidden)]
pub mod cli;
/// Render a [`model::Patch`] back to a unified diff, byte-for-byte for git-canonical input.
pub mod emit;
/// Application errors and the process exit codes they map to.
pub mod error;
/// The environment variables that decide which repository a `git` child acts on, and the
/// plumbing that feeds one a diff.
///
/// Not part of the library contract, and public for the same reason as [`cli`]: the test crates
/// insulate their `git` invocations exactly the way the tool does, and they cannot see a
/// `pub(crate)` item.
#[doc(hidden)]
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
