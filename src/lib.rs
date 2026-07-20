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
//! - [`list`] — enumerate addressable sub-hunks (human-readable and JSON);
//! - [`validate`] — check internal consistency of a result diff;
//! - [`subhunk_id`] — stable content ids for sub-hunks;
//! - [`emit`] — render a [`model::Patch`] back to a unified diff;
//! - [`error`] — application errors with process exit codes.
//!
//! See the `README` for the selector grammar and command reference.

pub mod cli;
pub mod emit;
pub mod error;
pub mod list;
pub mod model;
pub mod parser;
pub mod select;
pub mod split;
pub mod subhunk_id;
pub mod validate;
