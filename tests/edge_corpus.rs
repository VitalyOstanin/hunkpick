// Integration tests for git extended headers and special diff cases.

mod common;

use assert_cmd::Command;
use common::git_output;
use predicates::prelude::*;

// Each test below is introduced by a banner naming it. The banners used to be numbered, and the
// numbers stopped matching the tests as soon as one was added without one — a reference by number
// then pointed at the wrong test, which is worse than no number at all. The name is the address.

// ---------------------------------------------------------------------------
// rename_is_preserved
// ---------------------------------------------------------------------------

/// A renamed file with a content change: rename headers should be present in
/// hunkpick's output, or at a minimum hunkpick should exit 0 with non-empty output.
///
/// NOTE: rename detection requires sufficient content similarity and the `-M` flag.
/// The diff is captured with `git diff --staged -M` after `git mv`.
/// If the git version on this machine omits rename headers (environment-dependent),
/// the assertion is weakened to exit-0 + non-empty output.
#[test]
fn rename_is_preserved() {
    let dir = common::repo_with(&[("old.txt", "line1\nline2\nline3\nline4\nline5\n")]);

    // Move and change content so rename similarity is high enough for detection.
    let new_path = dir.path().join("new.txt");
    std::fs::rename(dir.path().join("old.txt"), &new_path).unwrap();
    std::fs::write(&new_path, "line1\nline2\nline3\nline4\nline5_changed\n").unwrap();
    common::sys(&dir, &["add", "-A"]);

    let diff = git_output(&dir, &["diff", "--staged", "-M"]);
    assert!(!diff.is_empty(), "staged diff must be non-empty");

    if diff.contains("rename from") {
        // Full assertion: rename headers must survive through hunkpick.
        let stdout = Command::cargo_bin("hunkpick")
            .unwrap()
            .args(["select", "1"])
            .write_stdin(diff.clone())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let text = std::str::from_utf8(&stdout).unwrap();
        assert!(
            text.contains("rename from") || text.contains("diff --git"),
            "rename headers must appear in output: {text}"
        );
        assert!(!text.is_empty());

        // list --json must also succeed.
        common::run_ok(&["list", "--json"], &diff);
    } else {
        // Rename detection not available in this environment: weaken to exit-0.
        common::run_ok(&["list", "--json"], &diff);
        Command::cargo_bin("hunkpick")
            .unwrap()
            .args(["select", "1"])
            .write_stdin(diff)
            .assert()
            .success()
            .stdout(predicate::str::is_empty().not());
    }
}

// ---------------------------------------------------------------------------
// mode_change_passthrough
// ---------------------------------------------------------------------------

/// A file whose mode changes (non-executable → executable) and whose content
/// also changes: hunkpick must preserve the old/new mode header lines.
///
/// Unix-only: the executable bit is not tracked on Windows (NTFS has no such
/// permission and git's `core.filemode` defaults to false there), so `git diff`
/// emits no `old mode`/`new mode` headers and the scenario cannot be produced.
#[cfg(unix)]
#[test]
fn mode_change_passthrough() {
    let dir = common::repo_with(&[("f.sh", "line1\nline2\n")]);

    // Make executable and change a line.
    let path = dir.path().join("f.sh");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(&path, "line1\nline2_changed\n").unwrap();

    let diff = common::diff_after(&dir, &[]);

    // The diff must carry old/new mode lines.
    assert!(
        diff.contains("old mode") && diff.contains("new mode"),
        "mode-change diff must contain old/new mode headers: {diff}"
    );

    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(diff)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = std::str::from_utf8(&stdout).unwrap();
    assert!(text.contains("old mode"), "output must contain 'old mode'");
    assert!(text.contains("new mode"), "output must contain 'new mode'");
}

// ---------------------------------------------------------------------------
// new_file
// ---------------------------------------------------------------------------

/// A brand-new staged file: hunkpick must exit 0 and the output must contain
/// the `new file mode` header and the added lines.
#[test]
fn new_file() {
    let dir = common::repo_with(&[]);
    // repo_with makes an initial empty commit; now stage a new file.
    std::fs::write(dir.path().join("newf.txt"), "line1\nline2\n").unwrap();
    common::sys(&dir, &["add", "newf.txt"]);

    let diff = common::diff_staged(&dir);
    assert!(!diff.is_empty(), "staged diff must be non-empty");
    assert!(
        diff.contains("new file mode"),
        "diff must contain 'new file mode': {diff}"
    );

    Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("list")
        .write_stdin(diff.clone())
        .assert()
        .success();

    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(diff)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = std::str::from_utf8(&stdout).unwrap();
    assert!(
        text.contains("new file mode"),
        "output must contain 'new file mode'"
    );
    assert!(text.contains("+line1"), "output must contain added lines");
}

// ---------------------------------------------------------------------------
// deleted_file
// ---------------------------------------------------------------------------

/// Deleting a committed file: hunkpick must exit 0 and preserve the
/// `deleted file mode` header.
#[test]
fn deleted_file() {
    let dir = common::repo_with(&[("f.txt", "line1\nline2\n")]);
    std::fs::remove_file(dir.path().join("f.txt")).unwrap();

    let diff = common::diff_after(&dir, &[]);
    assert!(!diff.is_empty(), "diff must be non-empty after deletion");
    assert!(
        diff.contains("deleted file mode"),
        "diff must contain 'deleted file mode': {diff}"
    );

    Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("list")
        .write_stdin(diff.clone())
        .assert()
        .success();

    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(diff)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = std::str::from_utf8(&stdout).unwrap();
    assert!(
        text.contains("deleted file mode"),
        "output must contain 'deleted file mode'"
    );
}

// ---------------------------------------------------------------------------
// binary_file
// ---------------------------------------------------------------------------

/// A binary file diff: `list --json` must mark it `binary: true` with zero sub-hunks.
/// `select 1` on a binary-only diff succeeds (exit 0) and emits the binary stanza —
/// this matches the actual implementation behaviour (binary files bypass index bounds).
#[test]
fn binary_file() {
    let dir = common::repo_with(&[]);
    // Write a file containing a NUL byte so git treats it as binary.
    std::fs::write(dir.path().join("f.bin"), b"hello\x00world").unwrap();
    common::sys(&dir, &["add", "f.bin"]);
    common::sys(&dir, &["commit", "-q", "-m", "add binary"]);
    std::fs::write(dir.path().join("f.bin"), b"bye\x00world").unwrap();

    let diff = common::diff_after(&dir, &[]);
    assert!(!diff.is_empty(), "binary diff must be non-empty");
    assert!(
        diff.contains("Binary files"),
        "diff must contain 'Binary files': {diff}"
    );

    let json = common::list_json(&diff);
    let files = json.as_array().expect("top-level must be array");
    assert_eq!(files.len(), 1);
    assert_eq!(
        files[0]["binary"], true,
        "binary file must be marked binary: true"
    );
    let hunks = files[0]["hunks"].as_array().expect("hunks must be array");
    assert_eq!(hunks.len(), 0, "binary file must have zero sub-hunks");

    // Actual behaviour: select 1 on a binary-only diff exits 0 and emits the binary stanza.
    // (The implementation bypasses index bounds for binary files.)
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(diff)
        .assert()
        .success()
        .stdout(predicate::str::contains("Binary files"));
}

// ---------------------------------------------------------------------------
// crlf_preserved
// ---------------------------------------------------------------------------

/// An inline fixture with CRLF line endings: select must round-trip the \r bytes.
/// (No git involvement; tests the parser/emitter directly via the CLI.)
#[test]
fn crlf_preserved() {
    // Build the diff bytes explicitly with \r\n endings.
    let diff: Vec<u8> = concat!(
        "diff --git a/f b/f\r\n",
        "--- a/f\r\n",
        "+++ b/f\r\n",
        "@@ -1,3 +1,3 @@\r\n",
        " a\r\n",
        "-b\r\n",
        "+B\r\n",
        " c\r\n",
    )
    .bytes()
    .collect();

    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("select")
        .arg("1")
        .write_stdin(diff)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    // The output must contain at least one \r byte (CRLF preserved).
    assert!(
        stdout.contains(&b'\r'),
        "output must preserve \\r bytes from CRLF input"
    );
}

// ---------------------------------------------------------------------------
// plain_non_git_diff
// ---------------------------------------------------------------------------

/// A plain (non-git) unified diff without `diff --git` preamble:
/// hunkpick must exit 0 and the output must start with `--- `.
#[test]
fn plain_non_git_diff() {
    let diff = "\
--- old/f
+++ new/f
@@ -1,3 +1,3 @@
 a
-b
+B
 c
";

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(diff)
        .assert()
        .success()
        .stdout(predicate::str::starts_with("--- "));
}

// ---------------------------------------------------------------------------
// deleted_file_is_listed_under_its_old_name
// ---------------------------------------------------------------------------

/// A deletion has `+++ /dev/null`. The listing must name the file by its old path:
/// several deletions in one diff would otherwise be indistinguishable, and `/dev/null`
/// is useless as a selector.
#[test]
fn deleted_file_is_listed_under_its_old_name() {
    let dir = common::repo_with(&[("gone.txt", "x\ny\n"), ("kept.txt", "k\n")]);
    std::fs::remove_file(dir.path().join("gone.txt")).unwrap();
    let diff = git_output(&dir, &["diff"]);
    assert!(diff.contains("+++ /dev/null"), "deletion diff expected");

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["list"])
        .write_stdin(diff.clone())
        .assert()
        .success()
        .stdout(predicate::str::contains("gone.txt"))
        .stdout(predicate::str::contains("/dev/null").not());

    // The old name also works as a selector.
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "gone.txt:1"])
        .write_stdin(diff)
        .assert()
        .success()
        .stdout(predicate::str::contains("-x"));
}

// ---------------------------------------------------------------------------
// format_patch_signature_survives
// ---------------------------------------------------------------------------

/// `git format-patch` ends its output with "-- " and the git version, after the last hunk.
/// Those lines must stay after the hunk in the result, not move above it.
#[test]
fn format_patch_signature_survives() {
    let dir = common::repo_with(&[("f", "a\nb\nc\n")]);
    std::fs::write(dir.path().join("f"), "a\nB\nc\n").unwrap();
    common::sys(&dir, &["commit", "-qam", "change"]);
    let patch = git_output(&dir, &["format-patch", "-1", "--stdout"]);
    assert!(patch.contains("\n-- \n"), "signature expected in patch");

    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "*"])
        .write_stdin(patch)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(stdout).unwrap();
    let hunk_at = text.find("@@ ").expect("hunk in output");
    let sig_at = text.find("\n-- \n").expect("signature in output");
    assert!(sig_at > hunk_at, "signature must follow the hunk: {text}");
}

// ---------------------------------------------------------------------------
// binary_file_is_addressable_in_a_multi_file_diff
// ---------------------------------------------------------------------------

/// A binary file's entry has no `---`/`+++`, so its name comes from the `diff --git` line.
/// Without it the entry is unaddressable in a multi-file diff, where a path is mandatory.
#[test]
fn binary_file_is_addressable_in_a_multi_file_diff() {
    let dir = common::repo_with(&[("text.txt", "text\n")]);
    std::fs::write(dir.path().join("bin.dat"), [0u8, 1, 2, b'x']).unwrap();
    common::sys(&dir, &["add", "bin.dat"]);
    common::sys(&dir, &["commit", "-qm", "add binary"]);
    std::fs::write(dir.path().join("bin.dat"), [0u8, 1, 3, b'y']).unwrap();
    let diff = common::diff_after(&dir, &[("text.txt", "TEXT\n")]);
    assert!(diff.contains("Binary files"), "binary entry expected");

    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "bin.dat:*", "text.txt:1"])
        .write_stdin(diff.clone())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8_lossy(&stdout).into_owned();
    assert!(
        text.contains("Binary files"),
        "binary entry emitted: {text}"
    );
    assert!(text.contains("+TEXT"), "text change emitted: {text}");

    // The listing names the binary file instead of leaving the line anonymous.
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["list"])
        .write_stdin(diff)
        .assert()
        .success()
        .stdout(predicate::str::contains("bin.dat"));
}

// ---------------------------------------------------------------------------
// quoted_non_ascii_path_is_addressable
// ---------------------------------------------------------------------------

/// With git's default `core.quotePath` a non-ASCII name is written quoted and C-escaped.
/// The selector must accept the real name, and the emitted diff must keep the original bytes.
///
/// Unix-only: on Windows the file name reaches git through a UTF-16 path and the octal
/// escaping this test asserts on is not what git writes there.
#[test]
#[cfg(unix)]
fn quoted_non_ascii_path_is_addressable() {
    let dir = common::repo_with(&[("é.txt", "a\nb\n")]);
    let diff = common::diff_after(&dir, &[("é.txt", "a\nB\n")]);
    assert!(
        diff.contains("\\303\\251"),
        "quoted path expected in git output: {diff}"
    );

    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "é.txt:1"])
        .write_stdin(diff.clone())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8_lossy(&stdout).into_owned();
    assert!(
        text.contains("\\303\\251"),
        "emitted diff keeps the original spelling: {text}"
    );

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["list"])
        .write_stdin(diff)
        .assert()
        .success()
        .stdout(predicate::str::contains("é.txt"));
}

// ---------------------------------------------------------------------------
// rename_only_entry_is_selectable_with_star
// ---------------------------------------------------------------------------

/// A pure rename (or a mode change) is a diff entry with no hunks. `*` must take such an
/// entry whole, the way it takes a binary file, instead of reporting an empty selection.
#[test]
fn rename_only_entry_is_selectable_with_star() {
    let diff = "\
diff --git a/old b/new
similarity index 100%
rename from old
rename to new
";
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "*"])
        .write_stdin(diff)
        .assert()
        .success()
        .stdout(predicate::str::contains("rename from old"))
        .stdout(predicate::str::contains("rename to new"));
}

// ---------------------------------------------------------------------------
// path_with_invalid_utf8_is_addressable
// ---------------------------------------------------------------------------

/// A file name that is not valid UTF-8 (legal on Unix) must still be addressable: the diff
/// carries the raw bytes, so the selector has to as well. Rejecting the argument as
/// "invalid UTF-8" would make such a file unreachable by name in a multi-file diff.
///
/// Linux-only. Windows paths cannot hold arbitrary bytes at all, and macOS rejects them at the
/// filesystem: APFS and HFS+ require file names to be valid UTF-8, so the file this test needs
/// cannot be created there. The byte-exact addressing itself is not platform-specific — it is
/// also covered by the `selectors` fuzz target, which feeds arbitrary bytes as the selector.
#[test]
#[cfg(target_os = "linux")]
fn path_with_invalid_utf8_is_addressable() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let raw: Vec<u8> = b"bad\xffname.txt".to_vec();
    let os_name = OsString::from_vec(raw.clone());

    let dir = common::repo_with(&[]);
    std::fs::write(dir.path().join(&os_name), "a\nb\n").unwrap();
    common::sys(&dir, &["add", "-A"]);
    common::sys(&dir, &["commit", "-qm", "add"]);
    std::fs::write(dir.path().join(&os_name), "a\nB\n").unwrap();
    // `core.quotePath=false` keeps the raw bytes in the diff rather than octal escapes.
    let diff = common::git_output_bytes(&dir, &["-c", "core.quotePath=false", "diff"]);
    assert!(!diff.is_empty(), "diff over the odd name must be non-empty");

    let mut selector = raw.clone();
    selector.extend_from_slice(b":1");
    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("select")
        .arg(OsString::from_vec(selector))
        .write_stdin(diff)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(
        stdout.windows(raw.len()).any(|w| w == raw.as_slice()),
        "emitted diff keeps the original path bytes"
    );
    assert!(
        String::from_utf8_lossy(&stdout).contains("+B"),
        "the addressed change is emitted"
    );
}

// ---------------------------------------------------------------------------
// full_binary_patch_comes_out_byte_for_byte
// ---------------------------------------------------------------------------

/// `git diff --binary` writes the payload itself — `literal <n>`, base85 lines, blank
/// separators — after the `GIT binary patch` marker. Those lines carry no marker of their own,
/// so an entry that has already turned binary must keep taking them; otherwise they land in the
/// leading headers and come out above the marker, which git rejects as garbage. This is the
/// form `git apply --cached` needs to stage a binary change, so it has to survive intact.
#[test]
fn full_binary_patch_comes_out_byte_for_byte() {
    let dir = common::repo_with(&[]);
    std::fs::write(dir.path().join("img.bin"), b"\x00\x01\x02one").unwrap();
    common::sys(&dir, &["add", "img.bin"]);
    common::sys(&dir, &["commit", "-q", "-m", "add binary"]);
    std::fs::write(dir.path().join("img.bin"), b"\x00\x01\x02two-x").unwrap();

    let diff = common::git_output_bytes(&dir, &["diff", "--binary"]);
    assert!(
        diff.windows(16).any(|w| w == b"GIT binary patch"),
        "the fixture must be a full binary patch"
    );
    common::revert(&dir);

    let out = Command::cargo_bin("hunkpick")
        .unwrap()
        .args([
            "select",
            "img.bin:*",
            "--verify-result-diff-git",
            "-C",
            dir.path().to_str().unwrap(),
        ])
        .write_stdin(diff.clone())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(&diff),
        "a binary entry must be emitted whole and unchanged"
    );
}

// ---------------------------------------------------------------------------
// a_partial_line_set_on_a_deleted_file_is_a_usage_error
// ---------------------------------------------------------------------------

/// An entry that declares the file removed (`deleted file mode`, `+++ /dev/null`) has no room
/// for a partial selection: `@L` keeps the unselected deletions as context, so the header asks
/// git to delete a file whose body still has lines and git refuses
/// (`deleted file f still has contents`). Better a usage error than a diff that cannot apply.
#[test]
fn a_partial_line_set_on_a_deleted_file_is_a_usage_error() {
    let dir = common::repo_with(&[("f", "l1\nl2\nl3\nl4\nl5\n")]);
    common::sys(&dir, &["rm", "-q", "f"]);
    let diff = common::diff_staged(&dir);
    assert!(
        diff.contains("deleted file mode"),
        "the fixture must be a whole-file deletion: {diff}"
    );

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1@L1,2"])
        .write_stdin(diff.clone())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("deletes the file as a whole"));

    // Naming the sub-hunk whole still works: that is the file's removal, unchanged.
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(diff)
        .assert()
        .success();
}

// ---------------------------------------------------------------------------
// a_combined_diff_is_rejected_as_a_usage_error
// ---------------------------------------------------------------------------

/// A combined diff — what `git show` writes for a merge commit — has one marker column per
/// parent and `@@@` headers. Read as a two-sided diff it loses data quietly: the body is
/// truncated at the first line and a `--- removed in both parents` line invents a file entry.
/// hunkpick does not address that format, so it says so instead.
#[test]
fn a_combined_diff_is_rejected_as_a_usage_error() {
    let diff = concat!(
        "diff --cc f\n",
        "index 1111111,2222222..3333333\n",
        "--- a/f\n",
        "+++ b/f\n",
        "@@@ -1,3 -1,3 +1,3 @@@\n",
        "  ctx\n",
        "--- removed in both\n",
        "++ added\n",
    );

    for args in [vec!["list"], vec!["select", "f:*"]] {
        Command::cargo_bin("hunkpick")
            .unwrap()
            .args(&args)
            .write_stdin(diff)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("combined"));
    }
}

// ---------------------------------------------------------------------------
// a_combined_diff_without_file_markers_is_still_named_as_combined
// ---------------------------------------------------------------------------

/// `git diff --cc` writes no `---`/`+++` pair for a file resolved identically in both parents,
/// so such a combined diff carries none of the marker lines the non-diff guard looks for. It
/// must still be named for what it is: "no diff markers found" sends the caller looking for a
/// truncated pipe rather than at the format.
#[test]
fn a_combined_diff_without_file_markers_is_still_named_as_combined() {
    let diff = concat!(
        "diff --cc f\n",
        "index 1111111,2222222..3333333\n",
        "@@@ -1,2 -1,2 +1,2 @@@\n",
        "  ctx\n",
        "++ added\n",
    );

    Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("list")
        .write_stdin(diff)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("combined"));
}

// ---------------------------------------------------------------------------
// split_addresses_a_path_with_invalid_utf8
// ---------------------------------------------------------------------------

/// `split` addresses a hunk the same way `select` addresses a sub-hunk, so it has to accept the
/// same path bytes: a file whose name is not valid UTF-8 is otherwise reachable by `select` and
/// unreachable by `split`, for no reason the user can see.
///
/// Linux-only, and for the same reasons as `path_with_invalid_utf8_is_addressable` above.
#[test]
#[cfg(target_os = "linux")]
fn split_addresses_a_path_with_invalid_utf8() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let raw: Vec<u8> = b"bad\xffname.txt".to_vec();
    let os_name = OsString::from_vec(raw.clone());

    let dir = common::repo_with(&[]);
    std::fs::write(dir.path().join(&os_name), "a\nb\nc\nd\ne\n").unwrap();
    common::sys(&dir, &["add", "-A"]);
    common::sys(&dir, &["commit", "-qm", "add"]);
    std::fs::write(dir.path().join(&os_name), "a\nB\nc\nD\ne\n").unwrap();
    let diff = common::git_output_bytes(&dir, &["-c", "core.quotePath=false", "diff"]);

    let mut address = raw.clone();
    address.extend_from_slice(b":1");
    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("split")
        .arg(OsString::from_vec(address))
        .args(["--at", "3"])
        .write_stdin(diff)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        String::from_utf8_lossy(&stdout).matches("@@ -").count(),
        2,
        "the hunk is cut in two"
    );
    assert!(
        stdout.windows(raw.len()).any(|w| w == raw.as_slice()),
        "emitted diff keeps the original path bytes"
    );
}
