// The test repositories must not inherit the developer's (or the CI runner's) git configuration.
//
// A single global setting is enough to change what `git diff` prints and thus what every other
// integration test asserts on: `diff.noprefix` drops the `a/`/`b/` path prefixes, `core.autocrlf`
// rewrites line endings, `diff.mnemonicPrefix` renames them. A suite that only passes on a clean
// machine reports the machine, not the code.
//
// Reproducing that needs a poisoned `GIT_CONFIG_GLOBAL`, and setting it in-process would mean
// `unsafe { env::set_var }` plus an invariant ("this binary holds one test") that nothing checks:
// adding a second test to this file would compile fine and turn a green suite into undefined
// behaviour under `cargo test`, which runs tests in threads. So the scenario is `#[ignore]`d and
// re-run as a child process with the variable set through `Command::env` — a safe API, and an
// arrangement that keeps working however many tests this file grows.

mod common;

/// The name libtest knows the scenario by, used to select it in the child run.
const SCENARIO: &str = "poisoned_global_config_scenario";

/// The scenario: with `GIT_CONFIG_GLOBAL` and `GIT_CONFIG_SYSTEM` pointing at a hostile
/// configuration, the helpers must
/// still produce the diff a clean machine produces. Ignored by default — on its own, with no
/// poisoned variable in the environment, it asserts nothing of interest.
#[test]
#[ignore = "run by ambient_git_config_does_not_reach_the_test_repositories"]
fn poisoned_global_config_scenario() {
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

/// Write a hostile git configuration, then run the scenario above in a child process that
/// inherits it through both variables git reads a configuration file from. Poisoning only the
/// global one would leave the system one — the second half of the isolation the helpers apply —
/// unproven, and a helper that dropped it would still pass.
#[test]
fn ambient_git_config_does_not_reach_the_test_repositories() {
    let cfg = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        cfg.path(),
        "[diff]\n\tnoprefix = true\n[core]\n\tautocrlf = true\n",
    )
    .unwrap();

    let exe = std::env::current_exe().expect("the test binary knows its own path");
    let out = std::process::Command::new(exe)
        .args(["--exact", "--ignored", "--test-threads", "1", SCENARIO])
        .env("GIT_CONFIG_GLOBAL", cfg.path())
        .env("GIT_CONFIG_SYSTEM", cfg.path())
        .output()
        .expect("re-running this test binary");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "the scenario failed under a poisoned git configuration:\n{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // A filter that matches nothing also exits 0, which would make this test vacuous.
    assert!(
        stdout.contains("1 passed"),
        "the child run did not execute the scenario:\n{stdout}"
    );
}
