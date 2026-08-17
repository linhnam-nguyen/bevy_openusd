use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use openusd::usd::Stage;
use tempfile::TempDir;
use usd_git::{GitRepository, Repository, RevisionSpec};

#[test]
fn historical_materialization_opens_through_openusd() {
    let repository_dir = create_repository();
    let repository = Repository::open(repository_dir.path()).expect("open temporary repository");
    let revision = repository
        .resolve_revision(&RevisionSpec::from("HEAD"))
        .expect("resolve historical fixture revision");
    let materialized_root = repository_dir.path().join("materialized");
    repository
        .materialize_revision(revision.id(), &materialized_root)
        .expect("materialize historical fixture revision");

    let root_layer = materialized_root.join("model.usda");
    let root_layer = root_layer.to_str().expect("temporary path is UTF-8");
    let stage = Stage::open(root_layer).expect("materialized historical stage opens");

    assert!(
        stage
            .prim(openusd::sdf::path("/World/HistoricalChild").unwrap())
            .is_valid()
            .expect("query materialized historical prim validity")
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

    fs::create_dir_all(directory.path().join("layers")).unwrap();
    fs::write(
        directory.path().join("model.usda"),
        b"#usda 1.0\n(\n    subLayers = [@layers/referenced.usda@]\n)\ndef Xform \"World\" {}\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("layers/referenced.usda"),
        b"#usda 1.0\nover \"World\" {\n    def Xform \"HistoricalChild\" {}\n}\n",
    )
    .unwrap();
    run_git(directory.path(), ["add", "."]);
    run_git(directory.path(), ["commit", "-m", "historical USD stage"]);
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
