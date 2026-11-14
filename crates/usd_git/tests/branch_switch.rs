use std::fs;

use project_support::{create_test_repository, run_git};
use usd_git::{BranchName, BranchSwitchOutcome, Error, GitRepository, Repository};

#[test]
fn reports_non_ignored_untracked_worktree_state() {
    let directory = create_test_repository();
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
    let directory = create_test_repository();
    run_git(directory.path(), &["branch", "feature/switch"]);
    run_git(directory.path(), &["checkout", "feature/switch"]);
    fs::write(directory.path().join("feature.usda"), b"#usda 1.0\n").unwrap();
    run_git(directory.path(), &["add", "feature.usda"]);
    run_git(directory.path(), &["commit", "-m", "feature branch"]);
    run_git(directory.path(), &["checkout", "main"]);

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
    let directory = create_test_repository();
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
    run_git(directory.path(), &["branch", "feature/dirty"]);
    assert!(matches!(
        repository.switch_branch(&BranchName::new("feature/dirty").unwrap()),
        Err(Error::DirtyWorkingTree)
    ));
}

#[test]
fn switching_to_the_current_branch_is_a_noop() {
    let directory = create_test_repository();
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

#[test]
fn ignored_cache_and_recovery_changes_keep_branch_switch_eligible() {
    let directory = create_test_repository();
    fs::write(
        directory.path().join(".gitignore"),
        ".usdhub/cache/\n.usdhub/recovery/\n",
    )
    .unwrap();
    run_git(directory.path(), &["add", ".gitignore"]);
    run_git(
        directory.path(),
        &["commit", "-m", "ignore local Project state"],
    );
    run_git(directory.path(), &["branch", "feature"]);
    fs::create_dir_all(directory.path().join(".usdhub/cache")).unwrap();
    fs::create_dir_all(directory.path().join(".usdhub/recovery")).unwrap();
    fs::write(directory.path().join(".usdhub/cache/object"), b"cache").unwrap();
    fs::write(
        directory.path().join(".usdhub/recovery/checkpoint"),
        b"recovery",
    )
    .unwrap();

    let mut repository = Repository::open(directory.path()).expect("open repository");
    assert!(
        !repository
            .working_tree_status()
            .expect("read ignored worktree status")
            .dirty
    );
    assert!(matches!(
        repository
            .switch_branch(&BranchName::new("feature").unwrap())
            .expect("switch with ignored local state"),
        BranchSwitchOutcome::Switched { .. }
    ));
}
