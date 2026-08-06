use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_lists_public_workflow_without_internal_helper() {
    let mut command = Command::cargo_bin("codex-deepseek-switcher").expect("binary");
    command
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("setup"))
        .stdout(predicate::str::contains("use"))
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("key"))
        .stdout(predicate::str::contains("credential").not());
}

#[test]
fn restoring_without_saved_state_is_a_safe_error() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut command = Command::cargo_bin("codex-deepseek-switcher").expect("binary");
    command
        .env("CODEX_HOME", directory.path())
        .args(["use", "codex"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no saved Codex selection exists"));
}

#[test]
fn rejects_relative_codex_home() {
    let mut command = Command::cargo_bin("codex-deepseek-switcher").expect("binary");
    command
        .env("CODEX_HOME", "relative/path")
        .args(["use", "codex"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "CODEX_HOME must be a non-empty absolute path",
        ));
}
