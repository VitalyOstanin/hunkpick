use clap::{Parser, Subcommand, ValueEnum};
use std::ffi::OsString;
use std::path::PathBuf;

/// Default maximum input size in bytes (64 MiB).
pub const DEFAULT_MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;

/// Shown after the options on `hunkpick --help` (the full, long help).
const AFTER_LONG_HELP: &str = "\
Examples:
  # List addressable sub-hunks: 1-based per-file index + 16-hex content id
  git diff src/main.rs | hunkpick list

  # Machine-readable listing (adds id_count: how many sub-hunks share an id)
  git diff src/main.rs | hunkpick list --json

  # Stage sub-hunks 1 and 3 of a single-file diff
  git diff src/main.rs | hunkpick select 1,3 | git apply --cached

  # Multi-file diff (git diff over several files): address sub-hunks per path.
  # A bare index needs a single-file diff; with many files every selector needs path:.
  git diff src/a.rs src/b.rs src/c.rs \
    | hunkpick select src/a.rs:1,3 src/c.rs:2-4 | git apply --cached

  # Every sub-hunk of a file (or of a single-file diff)
  git diff | hunkpick select src/main.rs:* | git apply --cached

  # Split original hunk 1 at new-file line 5 (cut point must be a context line)
  git diff src/lib.rs | hunkpick split 1 --at 5

  # Split a sub-hunk by individual changed lines (@L numbers the +/- lines 1..N,
  # see `list --json` changed_lines). @L keeps both leading and trailing context
  # so a subset applies with no boundary restriction (two exceptions; see
  # `hunkpick select --help`). Split an addition-only block of 120 lines across
  # commits, one piece per round. The re-diff shows only what is left, renumbered
  # from 1, so the second round asks for 1-30 rather than 91-120:
  git diff src/lib.rs | hunkpick select 1@L1-90 | git apply --cached
  git diff src/lib.rs | hunkpick select 1@L1-30 | git apply --cached

  # Separate a replacement's removals from its insertions: stage the deletions,
  # commit, then re-diff and stage the rest. The re-diff renumbers the remaining
  # lines, so 1@L1,2 then selects the additions.
  git diff src/lib.rs | hunkpick select 1@L1,2 | git apply --cached && git commit -m 'remove ...'
  git diff src/lib.rs | hunkpick select 1@L1,2 | git apply --cached && git commit -m 'add ...'

Content ids (@<id>):
  A 16-hex id per sub-hunk, shown by `list` and accepted by `select`. It is
  context-free, so it survives a re-diff: capture it once, reuse it across the
  whole diff -> stage -> re-diff loop. Full rules: `hunkpick select --help`.

  # Select by content id (stable across re-diffs)
  git diff | hunkpick select @8002dd73f0dfd2f4 | git apply --cached

  # In a multi-file diff an id still addresses its own file: the path is part of
  # the id, so the same edit in another file gets a different id.
  git diff src/a.rs src/b.rs src/c.rs | hunkpick select @8002dd73f0dfd2f4 | git apply --cached

  # Several ids at once (space-separated); mix with path: selectors freely.
  # Read the ids from `list --json` first (the machine-readable form), then select:
  git diff | hunkpick list --json
  git diff | hunkpick select @8002dd73f0dfd2f4 @bf7bdaaf30c1e2d4 src/lib.rs:2 | git apply --cached

  # Full loop: list ONCE, then stage groups by @id (one or more ids each),
  # re-running git diff every round. The ids from the single `list` stay valid
  # even as staging renumbers the bare indices, so the listing is never re-read.
  # `*` takes whatever sub-hunks are left at the end.
  git diff src/x.js | hunkpick list --json    # capture ids once (id_count flags shared ids)
  git diff src/x.js | hunkpick select @bf7bdaaf30c1e2d4 \
    | git apply --cached && git commit -m 'fix: ...'
  git diff src/x.js | hunkpick select @058b36528575a870 @399e1cd421e268cc \
    | git apply --cached && git commit -m 'feat: ...'
  git diff src/x.js | hunkpick select '*' | git apply --cached && git commit -m 'chore: ...'

Each subcommand has its own detailed --help (full selector grammar, content-id
rules, verification flags):
  hunkpick list --help | hunkpick select --help | hunkpick split --help";

/// Shown after the options on the short `hunkpick -h`.
const AFTER_SHORT_HELP: &str = "Run 'hunkpick --help' for examples and content-id usage.";

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
#[command(
    name = "hunkpick",
    version,
    after_help = AFTER_SHORT_HELP,
    after_long_help = AFTER_LONG_HELP
)]
pub struct Cli {
    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Verification options shared by `select` and `split` (both produce a result diff).
#[derive(clap::Args, Debug)]
pub struct VerifyOpts {
    /// Disable the default internal consistency check of the result diff.
    #[arg(long)]
    pub no_verify_result_diff_internal: bool,
    /// Additionally check the result with `git apply --check` against the WORKING TREE.
    ///
    /// Expert option. `git apply --check` reads the working tree, so it answers "does this
    /// diff apply to the files as they are on disk right now". In the usual staging pipeline
    /// (`git diff | hunkpick select ... | git apply --cached`) the working tree already holds
    /// those edits, so a correct result diff is reported as not applying (exit 70). Use it
    /// when the tree is at the state the diff is meant for -- reviewing a patch file, or with
    /// `-C DIR` pointing at such a tree -- not as a routine check of the staging loop.
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

// The help text for `select` uses `L<set>` as a placeholder metavariable. These `///`
// comments are surfaced verbatim by clap in `--help`, so wrapping the token in backticks or
// escaping the `<` would leak into the CLI output; suppress the rustdoc HTML-tag lint instead.
#[allow(rustdoc::invalid_html_tags)]
/// The subcommands hunkpick offers.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// List the addressable sub-hunks of each file.
    ///
    /// Each hunk is auto-split into minimal sub-hunks (one contiguous change run each). For
    /// every sub-hunk `list` shows a 1-based per-file index and a 16-hex content id; either
    /// can be passed to `select` (the id as `@<id>`). A sub-hunk that is all additions (a
    /// file-creation or pure-append block) is flagged with a `[+add]` marker in the human
    /// listing. `--json` emits the same data as a stable machine schema, plus `id_count` (how
    /// many sub-hunks share an id; 1 = unique), `addition_only` (that same all-additions flag),
    /// and `changed_lines` (each sub-hunk's +/- lines, 1-based in body order, for addressing
    /// with `select INDEX@L<set>`). Binary files are listed with no sub-hunks.
    ///
    /// The human listing escapes text a terminal would act on (escape sequences, control
    /// bytes, bidirectional overrides). The JSON listing does not: its text fields carry the
    /// diff's own content, so a consumer printing them to a terminal must escape them. Those
    /// fields are also lossy for non-UTF-8 bytes; address such a file by its content id.
    List {
        /// Emit machine-readable JSON instead of the human listing.
        #[arg(long)]
        json: bool,
        /// Colour the human listing: `auto` (a terminal, unless NO_COLOR is set), `always`
        /// or `never`. CLICOLOR_FORCE forces colour in `auto`. `--json` is never coloured.
        #[arg(long, value_enum, default_value_t = ColorMode::Auto)]
        color: ColorMode,
        /// Where the diff is read from.
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
        ///   path:N@L<set>   cut sub-hunk N of a file to a subset of its changed (+/-) lines
        ///   N@L<set>        the same in a single-file diff (set: e.g. L1,3 or L1-2,4)
        ///
        /// Indices and ids come from `list`. An id is derived from the file path and the
        /// sub-hunk's changed (+/-) lines only, so it survives a re-diff; it matches
        /// case-insensitively, and changes with identical +/- lines share one and are selected
        /// together (use path:N, guided by id_count from `list --json`, to address just one).
        /// Precedence: path:set first (a file named `@foo` stays addressable as `@foo:1`),
        /// then @ID, then a bare set.
        ///
        /// In INDEX@L<set> only a numeric index may precede '@' (not @id, not *), and the set
        /// numbers the sub-hunk's changed lines 1..N in body order: deletions and additions
        /// share one numbering, as shown by `list --json`'s changed_lines. The cut keeps both
        /// leading and trailing context, so a subset applies with no boundary restriction,
        /// except on an entry that deletes the file (+++ /dev/null), where a partial subset is
        /// a usage error, and for a piece left with no context at all (whole-file replacement,
        /// file creation/deletion), which git applies only with --unidiff-zero. Address a
        /// sub-hunk by @L once per invocation (do not combine it with another selection of the
        /// same sub-hunk); stage further pieces in later rounds.
        ///
        /// The README is the reference for the grammar, with worked examples of splitting a
        /// block across commits and separating a replacement's removals from its insertions.
        // OsString, not String: a diff can name a file whose path is not valid UTF-8 (legal on
        // Unix), and such a file must stay addressable by name. clap would reject the argument
        // outright as "invalid UTF-8" before hunkpick ever saw it.
        #[arg(verbatim_doc_comment)]
        selectors: Vec<OsString>,
        /// Where the diff is read from.
        #[command(flatten)]
        input: InputOpts,
        /// How the result diff is verified before it is written out.
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
        /// hunks (before auto-split), not the sub-hunk indices shown by `list`. Taken as raw
        /// bytes, like a selector, so a path that is not valid UTF-8 can be spelled.
        hunk: OsString,
        /// New-file line numbers to cut at.
        #[arg(long = "at", value_delimiter = ',', required = true)]
        at: Vec<u32>,
        /// Where the diff is read from.
        #[command(flatten)]
        input: InputOpts,
        /// How the result diff is verified before it is written out.
        #[command(flatten)]
        verify: VerifyOpts,
    },
}

/// When to colorize the human-readable listing.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum ColorMode {
    /// Colorize when stdout is a terminal, honouring `NO_COLOR` and `CLICOLOR_FORCE`.
    Auto,
    /// Always colorize, even into a pipe.
    Always,
    /// Never colorize.
    Never,
}

/// Resolve whether to colorize, based on the mode, stdout TTY state, `NO_COLOR` and
/// `CLICOLOR_FORCE`. An environment variable counts as set only when its value is non-empty
/// (per the `NO_COLOR` and CLICOLOR conventions: a bare or empty value is ignored).
pub fn resolve_color(mode: ColorMode) -> bool {
    let is_tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let no_color = env_flag_set("NO_COLOR");
    let clicolor_force = env_flag_set("CLICOLOR_FORCE");
    resolve_color_with(mode, is_tty, no_color, clicolor_force)
}

/// True when environment variable `name` is set to a non-empty value.
fn env_flag_set(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty())
}

/// The decision behind [`resolve_color`], with the environment passed in so it can be tested.
pub fn resolve_color_with(
    mode: ColorMode,
    is_tty: bool,
    no_color: bool,
    clicolor_force: bool,
) -> bool {
    match mode {
        ColorMode::Always => true,
        ColorMode::Never => false,
        // `CLICOLOR_FORCE` forces color on a non-tty (e.g. a pipe); `NO_COLOR` still wins so a
        // hard opt-out is always honoured.
        ColorMode::Auto => (is_tty || clicolor_force) && !no_color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use clap::Parser;

    #[test]
    fn never_disables_color() {
        assert!(!resolve_color_with(ColorMode::Never, true, false, false));
    }

    #[test]
    fn always_enables_even_without_tty() {
        assert!(resolve_color_with(ColorMode::Always, false, false, false));
    }

    #[test]
    fn auto_follows_tty_unless_no_color() {
        assert!(resolve_color_with(ColorMode::Auto, true, false, false));
        assert!(!resolve_color_with(ColorMode::Auto, true, true, false));
        assert!(!resolve_color_with(ColorMode::Auto, false, false, false));
    }

    #[test]
    fn auto_clicolor_force_enables_on_non_tty() {
        // CLICOLOR_FORCE turns color on when stdout is not a tty...
        assert!(resolve_color_with(ColorMode::Auto, false, false, true));
        // ...but NO_COLOR still wins over CLICOLOR_FORCE.
        assert!(!resolve_color_with(ColorMode::Auto, false, true, true));
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
                assert_eq!(selectors, vec![OsString::from("1,3")]);
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

    #[test]
    fn long_help_documents_lineset_form() {
        let mut cmd = Cli::command();
        let help = cmd.render_long_help().to_string();
        assert!(
            help.contains("1@L1-90"),
            "long help must show an @L changed-line example"
        );
    }

    /// Every help text this binary can print stays ASCII. Rust writes stdout as UTF-8, and a
    /// Windows console on a non-UTF-8 code page (cp866/cp1251 are the defaults in several
    /// locales) renders anything else as mojibake in the middle of the help — and the release
    /// pipeline ships an x86_64-pc-windows-msvc binary. Typographic punctuation is the way
    /// this creeps in, so the guard is a test rather than a review habit.
    #[test]
    fn every_help_text_is_ascii() {
        let mut cmd = Cli::command();
        let mut texts = vec![cmd.render_long_help().to_string()];
        for sub in cmd.get_subcommands_mut() {
            texts.push(sub.render_long_help().to_string());
        }
        for help in texts {
            let offender = help.chars().find(|c| !c.is_ascii());
            assert!(
                offender.is_none(),
                "help text contains a non-ASCII character {:?}; use ASCII punctuation",
                offender.unwrap()
            );
        }
    }
}
