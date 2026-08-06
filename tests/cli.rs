use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_lists_public_workflow_without_internal_helper() {
    let mut command = Command::cargo_bin("deepswitch").expect("binary");
    command
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("codex"))
        .stdout(predicate::str::contains("deepseek"))
        .stdout(predicate::str::contains("setup"))
        .stdout(predicate::str::contains("use").not())
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("key"))
        .stdout(predicate::str::contains("credential").not());
}

#[test]
fn restoring_without_saved_state_is_a_safe_error() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut command = Command::cargo_bin("deepswitch").expect("binary");
    command
        .env("CODEX_HOME", directory.path())
        .arg("codex")
        .assert()
        .failure()
        .stderr(predicate::str::contains("no saved Codex selection exists"));
}

#[test]
fn no_command_prompts_for_provider() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut command = Command::cargo_bin("deepswitch").expect("binary");
    command
        .env("CODEX_HOME", directory.path())
        .write_stdin("1\n")
        .assert()
        .failure()
        .stdout(predicate::str::contains("Choose a provider"))
        .stdout(predicate::str::contains("1) Codex"))
        .stdout(predicate::str::contains("2) DeepSeek"))
        .stderr(predicate::str::contains("no saved Codex selection exists"));
}

#[test]
fn rejects_relative_codex_home() {
    let mut command = Command::cargo_bin("deepswitch").expect("binary");
    command
        .env("CODEX_HOME", "relative/path")
        .arg("codex")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "CODEX_HOME must be a non-empty absolute path",
        ));
}
