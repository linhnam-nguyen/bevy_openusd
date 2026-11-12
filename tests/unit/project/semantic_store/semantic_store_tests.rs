use std::{fs, path::Path, process::Command};

use usd_git::GitRepository;

use super::regenerate_git_snapshot;

fn git(repository: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repository)
        .args(args)
        .output()
        .expect("git should be installed for the regeneration test");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn materialized_git_revision_regenerates_a_snapshot() {
    let repository_dir = tempfile::tempdir().expect("repository tempdir should create");
    git(repository_dir.path(), &["init"]);
    git(
        repository_dir.path(),
        &["config", "user.name", "USDHub Tests"],
    );
    git(
        repository_dir.path(),
        &["config", "user.email", "tests@usdhub.invalid"],
    );
    fs::write(
        repository_dir.path().join("model.usda"),
        include_bytes!("../../../../crates/usd_semantic/tests/fixtures/identity_original.usda"),
    )
    .expect("USD fixture should write");
    git(repository_dir.path(), &["add", "model.usda"]);
    git(repository_dir.path(), &["commit", "-m", "initial"]);

    let repository = usd_git::Repository::open(repository_dir.path()).expect("repository opens");
    let revision = repository
        .head()
        .expect("HEAD resolves")
        .expect("HEAD exists");
    let destination = tempfile::tempdir().expect("materialization tempdir should create");
    let snapshot = regenerate_git_snapshot(
        repository_dir.path(),
        revision.id(),
        destination.path().join("tree").as_path(),
        Path::new("model.usda"),
        usd_semantic::SemanticConfig::default(),
    )
    .expect("materialized stage should regenerate");

    assert_eq!(
        snapshot.source,
        usd_model::SnapshotSource::GitCommit {
            oid: revision.id().to_string()
        }
    );
    assert!(!snapshot.entities.is_empty());
}
