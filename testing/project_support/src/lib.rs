//! Shared Project & Git test support: temporary repositories, Git CLI harnesses, and branch utilities.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

/// Query the configured Git default initial branch, falling back to "main".
pub fn configured_initial_branch() -> String {
    let directory = tempfile::tempdir().expect("create temporary directory for Git config");
    let output = Command::new("git")
        .args(["config", "--default", "main", "init.defaultBranch"])
        .current_dir(directory.path())
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
    let directory = tempfile::tempdir().expect("create temporary directory");
    run_git(directory.path(), &["init", "-b", "main"]);
    run_git(directory.path(), &["config", "user.name", "USDHub Test"]);
    run_git(
        directory.path(),
        &["config", "user.email", "test@usdhub.invalid"],
    );
    fs::write(directory.path().join("model.usda"), b"#usda 1.0\n").unwrap();
    run_git(directory.path(), &["add", "model.usda"]);
    run_git(directory.path(), &["commit", "-m", "initial USD stage"]);
    directory
}

/// Create a temporary layered Git repository with model.usda and layers/referenced.usda.
pub fn create_layered_test_repository() -> TempDir {
    let directory = tempfile::tempdir().expect("create temporary directory");
    run_git(directory.path(), &["init", "-b", "main"]);
    run_git(directory.path(), &["config", "user.name", "USDHub Test"]);
    run_git(
        directory.path(),
        &["config", "user.email", "test@usdhub.invalid"],
    );

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_configured_initial_branch() {
        let branch = configured_initial_branch();
        assert!(!branch.is_empty(), "initial branch must not be empty");
        assert!(
            branch == "main" || branch == "master",
            "branch was: {branch}"
        );
    }

    #[test]
    fn test_create_test_repository() {
        let repo = create_test_repository();
        assert!(repo.path().join("model.usda").is_file());
        let output = run_git(repo.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);
        assert!(output.status.success());
    }

    #[test]
    fn test_create_layered_test_repository() {
        let repo = create_layered_test_repository();
        assert!(repo.path().join("model.usda").is_file());
        assert!(repo.path().join("layers/referenced.usda").is_file());
        let content = fs::read(repo.path().join("model.usda")).unwrap();
        assert_eq!(content, usd_support::MODEL_USDA);
    }
}
