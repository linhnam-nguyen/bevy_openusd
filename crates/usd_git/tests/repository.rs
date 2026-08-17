use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use tempfile::TempDir;
use usd_git::{CommitRequest, Error, GitRepository, Repository, RevisionSpec};

const MODEL_USDA: &[u8] =
    include_bytes!("../../usd_semantic/tests/fixtures/identity_original.usda");

#[test]
fn resolves_history_metadata_and_materializes_the_complete_tree() {
    let repository_dir = create_repository();
    let repository = Repository::open(repository_dir.path()).expect("open temporary repository");

    let head = repository
        .head()
        .expect("resolve HEAD")
        .expect("temporary repository has a commit");
    let branch = repository
        .resolve_revision(&RevisionSpec::from("main"))
        .expect("resolve main branch");
    assert_eq!(head, branch);

    let commit = repository
        .read_commit(head.id())
        .expect("read commit metadata");
    assert_eq!(commit.id, *head.id());
    assert!(commit.parents.is_empty());
    assert_eq!(commit.author.name, "USDHub Test");
    assert_eq!(commit.author.email, "test@usdhub.invalid");
    assert_eq!(commit.committer.name, "USDHub Test");
    assert_eq!(commit.message, "initial USD stage\n");

    let materialized_root = repository_dir.path().join("materialized");
    let materialized = repository
        .materialize_revision(head.id(), &materialized_root)
        .expect("materialize complete revision tree");
    assert_eq!(materialized.revision, *head.id());
    assert_eq!(materialized.root, materialized_root);
    assert_eq!(materialized.file_count, 2);
    assert_eq!(
        fs::read(materialized.root.join("model.usda")).unwrap(),
        MODEL_USDA
    );
    assert_eq!(
        fs::read(materialized.root.join("layers/referenced.usda")).unwrap(),
        b"#usda 1.0\n"
    );
}

#[test]
fn resolves_and_materializes_a_historical_parent() {
    let repository_dir = create_repository();
    fs::write(
        repository_dir.path().join("model.usda"),
        b"#usda 1.0\n(def \"stage\") {}\n",
    )
    .unwrap();
    run_git(repository_dir.path(), ["add", "."]);
    run_git(repository_dir.path(), ["commit", "-m", "second USD stage"]);

    let repository = Repository::open(repository_dir.path()).expect("open temporary repository");
    let historical = repository
        .resolve_revision(&RevisionSpec::from("HEAD~1"))
        .expect("resolve historical parent");
    let materialized_root = repository_dir.path().join("historical");
    repository
        .materialize_revision(historical.id(), &materialized_root)
        .expect("materialize historical parent");

    assert_eq!(
        fs::read(materialized_root.join("model.usda")).unwrap(),
        MODEL_USDA
    );
    assert_eq!(
        fs::read(materialized_root.join("layers/referenced.usda")).unwrap(),
        b"#usda 1.0\n"
    );
}

#[test]
fn commit_creation_is_explicitly_read_only() {
    let repository_dir = create_repository();
    let mut repository =
        Repository::open(repository_dir.path()).expect("open temporary repository");
    let before = repository.head().unwrap().unwrap();

    let result = repository.create_commit(CommitRequest::new(
        "should not be created in milestone 10",
        repository_dir.path(),
    ));
    assert!(matches!(result, Err(Error::ReadOnly)));

    let after = repository.head().unwrap().unwrap();
    assert_eq!(before, after);
}

fn create_repository() -> TempDir {
    let directory = tempfile::tempdir().expect("create temporary directory");
    run_git(directory.path(), ["init", "-b", "main"]);
    run_git(directory.path(), ["config", "user.name", "USDHub Test"]);
    run_git(
        directory.path(),
        ["config", "user.email", "test@usdhub.invalid"],
    );

    fs::create_dir_all(directory.path().join("layers")).unwrap();
    fs::write(directory.path().join("model.usda"), MODEL_USDA).unwrap();
    fs::write(
        directory.path().join("layers/referenced.usda"),
        b"#usda 1.0\n",
    )
    .unwrap();
    run_git(directory.path(), ["add", "."]);
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
