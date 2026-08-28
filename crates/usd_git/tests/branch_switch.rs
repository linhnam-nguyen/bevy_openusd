use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use tempfile::TempDir;
use usd_git::{BranchName, BranchSwitchOutcome, Error, GitRepository, Repository};

#[test]
fn reports_non_ignored_untracked_worktree_state() {
    let directory = create_repository();
    let repository = Repository::open(directory.path()).expect("open repository");

    fs::write(directory.path().join("untracked.usda"), b"#usda 1.0\n").unwrap();
    assert!(
        repository
            .working_tree_status()
            .expect("read worktree status")
            .dirty
    );
}

#[test]
fn switches_existing_local_branch_without_stashing_or_discarding_work() {
    let directory = create_repository();
    run_git(directory.path(), ["branch", "feature/switch"]);
    run_git(directory.path(), ["checkout", "feature/switch"]);
    fs::write(directory.path().join("feature.usda"), b"#usda 1.0\n").unwrap();
    run_git(directory.path(), ["add", "feature.usda"]);
    run_git(directory.path(), ["commit", "-m", "feature branch"]);
    run_git(directory.path(), ["checkout", "main"]);

    let mut repository = Repository::open(directory.path()).expect("open repository");
    assert_eq!(
        repository
            .switch_branch(&BranchName::new("feature/switch").unwrap())
            .expect("switch clean worktree"),
        BranchSwitchOutcome::Switched {
            from: Some("main".to_owned()),
            to: "feature/switch".to_owned(),
        }
    );
    assert_eq!(
        repository.current_branch().unwrap().as_deref(),
        Some("feature/switch")
    );
    assert!(directory.path().join("feature.usda").is_file());
}

#[test]
fn branch_switch_rejects_invalid_missing_and_dirty_requests() {
    let directory = create_repository();
    let mut repository = Repository::open(directory.path()).expect("open repository");

    assert!(matches!(
        BranchName::new("bad branch"),
        Err(Error::InvalidBranchName(_))
    ));
    assert!(matches!(
        repository.switch_branch(&BranchName::new("missing").unwrap()),
        Err(Error::BranchNotFound(name)) if name == "missing"
    ));

    fs::write(directory.path().join("untracked.usda"), b"dirty\n").unwrap();
    run_git(directory.path(), ["branch", "feature/dirty"]);
    assert!(matches!(
        repository.switch_branch(&BranchName::new("feature/dirty").unwrap()),
        Err(Error::DirtyWorkingTree)
    ));
}

#[test]
fn switching_to_the_current_branch_is_a_noop() {
    let directory = create_repository();
    let mut repository = Repository::open(directory.path()).expect("open repository");

    assert_eq!(
        repository
            .switch_branch(&BranchName::new("main").unwrap())
            .unwrap(),
        BranchSwitchOutcome::Unchanged {
            branch: "main".to_owned()
        }
    );
}

fn create_repository() -> TempDir {
    let directory = tempfile::tempdir().expect("create temporary directory");
    run_git(directory.path(), ["init", "-b", "main"]);
    run_git(directory.path(), ["config", "user.name", "USDHub Test"]);
    run_git(
        directory.path(),
        ["config", "user.email", "test@usdhub.invalid"],
    );
    fs::write(directory.path().join("model.usda"), b"#usda 1.0\n").unwrap();
    run_git(directory.path(), ["add", "model.usda"]);
    run_git(directory.path(), ["commit", "-m", "initial USD stage"]);
    directory
}

fn run_git<const N: usize>(directory: &Path, args: [&str; N]) -> Output {
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
