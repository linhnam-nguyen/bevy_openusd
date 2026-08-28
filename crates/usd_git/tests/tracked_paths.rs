use std::{fs, path::Path, process::Command};

use tempfile::TempDir;
use usd_git::{GitRepository, Repository};

const MODEL_USDA: &[u8] =
    include_bytes!("../../usd_semantic/tests/fixtures/identity_original.usda");

#[test]
fn detects_tracked_paths_below_a_git_relative_prefix() {
    let directory = create_repository();
    fs::create_dir_all(directory.path().join(".usdhub/cache")).unwrap();
    fs::create_dir_all(directory.path().join(".usdhub/recovery")).unwrap();
    fs::write(directory.path().join(".usdhub/cache/object"), b"cache").unwrap();
    fs::write(
        directory.path().join(".usdhub/recovery/session"),
        b"recovery",
    )
    .unwrap();
    run_git(directory.path(), ["add", "."]);
    run_git(directory.path(), ["commit", "-m", "track derived state"]);

    let repository = Repository::open(directory.path()).expect("open tracked repository");
    assert!(
        repository
            .has_tracked_path_prefix(".usdhub/cache")
            .expect("check tracked cache")
    );
    assert!(
        repository
            .has_tracked_path_prefix(".usdhub/recovery")
            .expect("check tracked recovery")
    );
    assert!(
        !repository
            .has_tracked_path_prefix(".usdhub/models")
            .expect("check untracked model prefix")
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

fn run_git<const N: usize>(directory: &Path, args: [&str; N]) {
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
}
