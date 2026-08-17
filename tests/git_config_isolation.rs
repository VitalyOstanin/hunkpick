// The test repositories must not inherit the developer's (or the CI runner's) git configuration.
//
// A single global setting is enough to change what `git diff` prints and thus what every other
// integration test asserts on: `diff.noprefix` drops the `a/`/`b/` path prefixes, `core.autocrlf`
// rewrites line endings, `diff.mnemonicPrefix` renames them. A suite that only passes on a clean
// machine reports the machine, not the code.
//
// This file holds a single test on purpose: it sets a process-wide environment variable, which is
// only safe while nothing else runs in the same binary.

mod common;

#[test]
fn ambient_global_git_config_does_not_reach_the_test_repositories() {
    let cfg = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        cfg.path(),
        "[diff]\n\tnoprefix = true\n[core]\n\tautocrlf = true\n",
    )
    .unwrap();
    // Inherited by every git process the helpers spawn unless they suppress it.
    std::env::set_var("GIT_CONFIG_GLOBAL", cfg.path());

    let dir = common::repo_with(&[("f", "a\nb\nc\n")]);
    let diff = common::diff_after(&dir, &[("f", "a\nB\nc\n")]);

    assert!(
        diff.contains("--- a/f") && diff.contains("+++ b/f"),
        "path prefixes must survive an ambient diff.noprefix: {diff}"
    );
    assert!(
        !diff.contains('\r'),
        "an ambient core.autocrlf must not reach the diff: {diff:?}"
    );
}
