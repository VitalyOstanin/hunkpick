use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Default maximum input size in bytes (64 MiB).
pub const DEFAULT_MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;

/// Pick and split unified-diff hunks.
///
/// hunkpick is a non-interactive filter: it reads a unified diff from stdin (or `-i FILE`)
/// and writes a diff to stdout. It never runs `git diff` itself, so it works with any diff
/// source (git, Mercurial, SVN, plain `diff -u`). Typical pipeline:
/// `git diff <path> | hunkpick select <selectors...> | git apply --cached`.
///
/// Each hunk is auto-split into minimal sub-hunks (one contiguous change run each). Use
/// `list` to see the addressable sub-hunks, `select` to emit a chosen subset, and `split`
/// to cut one hunk at given lines. Run `hunkpick <command> --help` for selector syntax,
/// content ids, and verification flags.
#[derive(Parser, Debug)]
#[command(name = "hunkpick", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Verification options shared by `select` and `split` (both produce a result diff).
#[derive(clap::Args, Debug)]
pub struct VerifyOpts {
    /// Disable the default internal consistency check of the result diff.
    #[arg(long)]
    pub no_verify_result_diff_internal: bool,
    /// Additionally verify the result diff applies via `git apply --check`.
    #[arg(long)]
    pub verify_result_diff_git: bool,
    /// Working tree directory for the git verification (default: current dir).
    /// Requires --verify-result-diff-git.
    #[arg(short = 'C', value_name = "DIR", requires = "verify_result_diff_git")]
    pub dir: Option<PathBuf>,
}

/// Input source and size limit, shared by all subcommands.
#[derive(clap::Args, Debug)]
pub struct InputOpts {
    /// Read the diff from FILE instead of stdin (`-` means stdin).
    #[arg(short = 'i', long = "input", value_name = "FILE")]
    pub input: Option<PathBuf>,
    /// Maximum input size in bytes; 0 disables the limit.
    #[arg(long = "max-input-bytes", value_name = "N", default_value_t = DEFAULT_MAX_INPUT_BYTES)]
    pub max_input_bytes: u64,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// List the addressable sub-hunks of each file.
    ///
    /// Each hunk is auto-split into minimal sub-hunks (one contiguous change run each). For
    /// every sub-hunk `list` shows a 1-based per-file index and a 16-hex content id; either
    /// can be passed to `select` (the id as `@<id>`). `--json` emits the same data as a
    /// stable machine schema, plus `id_count` (how many sub-hunks share an id; 1 = unique).
    /// Binary files are listed with no sub-hunks.
    List {
        /// Emit machine-readable JSON instead of the human listing.
        #[arg(long)]
        json: bool,
        #[arg(long, value_enum, default_value_t = ColorMode::Auto)]
        color: ColorMode,
        #[command(flatten)]
        input: InputOpts,
    },
    /// Emit only the selected sub-hunks as a unified diff.
    ///
    /// Pipe the result into `git apply --cached` to stage exactly those changes. A binary
    /// file named by any selector is emitted whole.
    Select {
        /// Sub-hunk selectors (one or more). Forms:
        ///
        ///   N | N,M | A-B   1-based index/range within a file (bare: single-file diff only)
        ///   path:N,M        the same, within the named file
        ///   path:* | *      every sub-hunk (of `path`, or of a single-file diff)
        ///   @ID             every sub-hunk whose 16-hex content id is ID (from `list`)
        ///
        /// Indices and ids come from `list`. A content id is derived from the file path and
        /// the sub-hunk's changed (+/-) lines only, ignoring context and the @@ line numbers:
        /// it stays the same across a re-diff even when an edit elsewhere shifts this change's
        /// line numbers or staging a neighbour rewrites its context, so an @ID captured once
        /// keeps addressing the change; it changes only when this change's own +/- lines do.
        /// Ids match case-insensitively. Changes with identical +/- lines share an id and are
        /// selected together; use path:N (guided by id_count from `list --json`) to address
        /// just one. Precedence: path:set first (a file named `@foo` stays addressable as
        /// `@foo:1`), then @ID, then a bare set.
        #[arg(verbatim_doc_comment)]
        selectors: Vec<String>,
        #[command(flatten)]
        input: InputOpts,
        #[command(flatten)]
        verify: VerifyOpts,
    },
    /// Explicitly split one hunk at given new-file line numbers (context lines only).
    ///
    /// Replaces one ORIGINAL hunk with the pieces produced by cutting it at `--at`. Unlike
    /// `select`, the address indexes the file's original hunks (before auto-split), and
    /// neither `*` nor `@id` is accepted.
    Split {
        /// Hunk address: `path:N` or `N` (single-file input). N indexes the file's ORIGINAL
        /// hunks (before auto-split), not the sub-hunk indices shown by `list`.
        hunk: String,
        /// New-file line numbers to cut at.
        #[arg(long = "at", value_delimiter = ',', required = true)]
        at: Vec<u32>,
        #[command(flatten)]
        input: InputOpts,
        #[command(flatten)]
        verify: VerifyOpts,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

/// Resolve whether to colorize, based on the mode, stdout TTY state and NO_COLOR.
pub fn resolve_color(mode: ColorMode) -> bool {
    let is_tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let no_color = std::env::var_os("NO_COLOR").is_some();
    resolve_color_with(mode, is_tty, no_color)
}

pub fn resolve_color_with(mode: ColorMode, is_tty: bool, no_color: bool) -> bool {
    match mode {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => is_tty && !no_color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn never_disables_color() {
        assert!(!resolve_color_with(ColorMode::Never, true, false));
    }

    #[test]
    fn always_enables_even_without_tty() {
        assert!(resolve_color_with(ColorMode::Always, false, false));
    }

    #[test]
    fn auto_follows_tty_unless_no_color() {
        assert!(resolve_color_with(ColorMode::Auto, true, false));
        assert!(!resolve_color_with(ColorMode::Auto, true, true));
        assert!(!resolve_color_with(ColorMode::Auto, false, false));
    }

    #[test]
    fn split_with_verify_flags_parses() {
        let cli = Cli::try_parse_from([
            "hunkpick",
            "split",
            "f",
            "--at",
            "3,5",
            "--verify-result-diff-git",
            "-C",
            "/tmp",
        ])
        .unwrap();
        match cli.command {
            Command::Split {
                hunk, at, verify, ..
            } => {
                assert_eq!(hunk, "f");
                assert_eq!(at, vec![3, 5]);
                assert!(verify.verify_result_diff_git);
                assert_eq!(verify.dir.as_deref(), Some(std::path::Path::new("/tmp")));
            }
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn select_no_internal_flag_parses() {
        let cli = Cli::try_parse_from([
            "hunkpick",
            "select",
            "1,3",
            "--no-verify-result-diff-internal",
        ])
        .unwrap();
        match cli.command {
            Command::Select {
                selectors, verify, ..
            } => {
                assert_eq!(selectors, vec!["1,3".to_string()]);
                assert!(verify.no_verify_result_diff_internal);
            }
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn dash_c_without_git_flag_is_rejected_by_clap() {
        // -C requires --verify-result-diff-git; clap must reject this.
        let res = Cli::try_parse_from(["hunkpick", "select", "1", "-C", "/tmp"]);
        assert!(res.is_err());
    }
}
