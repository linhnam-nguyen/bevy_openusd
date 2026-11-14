//! Shared Project & Git test support: temporary repositories, Git CLI harnesses, and branch utilities.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

/// Query the configured Git default initial branch, falling back to "main".
pub fn configured_initial_branch() -> String {
    let directory = tempfile::tempdir().expect("create temporary directory for Git config");
    read_initial_branch(Command::new("git").current_dir(directory.path()))
}

fn read_initial_branch(command: &mut Command) -> String {
    let output = command
        .args(["config", "--default", "main", "init.defaultBranch"])
        .output()
        .expect("read configured Git default branch");
    assert!(
        output.status.success(),
        "read configured Git default branch: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("Git default branch is UTF-8")
        .trim()
        .to_owned()
}

/// Execute a Git command in the given directory and assert that it succeeded.
pub fn run_git(directory: &Path, args: &[&str]) -> Output {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run git command");
    assert!(
        output.status.success(),
        "git command failed: {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Create a standard temporary Git repository with an initial commit.
pub fn create_test_repository() -> TempDir {
    let directory = init_test_repository();
    fs::write(directory.path().join("model.usda"), b"#usda 1.0\n").unwrap();
    run_git(directory.path(), &["add", "model.usda"]);
    run_git(directory.path(), &["commit", "-m", "initial USD stage"]);
    directory
}

/// Create a temporary layered Git repository with model.usda and layers/referenced.usda.
pub fn create_layered_test_repository() -> TempDir {
    let directory = init_test_repository();

    fs::create_dir_all(directory.path().join("layers")).unwrap();
    fs::write(directory.path().join("model.usda"), usd_support::MODEL_USDA).unwrap();
    fs::write(
        directory.path().join("layers/referenced.usda"),
        b"#usda 1.0\n",
    )
    .unwrap();
    run_git(directory.path(), &["add", "."]);
    run_git(directory.path(), &["commit", "-m", "initial USD stage"]);
    directory
}

fn init_test_repository() -> TempDir {
    let directory = tempfile::tempdir().expect("create temporary directory");
    run_git(directory.path(), &["init", "-b", "main"]);
    run_git(directory.path(), &["config", "user.name", "USDHub Test"]);
    run_git(
        directory.path(),
        &["config", "user.email", "test@usdhub.invalid"],
    );
    directory
}

#[cfg(test)]
#[path = "tests/unit/self_tests.rs"]
mod tests;
