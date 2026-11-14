use std::fs;

use project_support::{create_layered_test_repository, run_git};
use usd_git::{GitRepository, Repository};

#[test]
fn detects_tracked_paths_below_a_git_relative_prefix() {
    let directory = create_layered_test_repository();
    fs::create_dir_all(directory.path().join(".usdhub/cache")).unwrap();
    fs::create_dir_all(directory.path().join(".usdhub/recovery")).unwrap();
    fs::write(directory.path().join(".usdhub/cache/object"), b"cache").unwrap();
    fs::write(
        directory.path().join(".usdhub/recovery/session"),
        b"recovery",
    )
    .unwrap();
    run_git(directory.path(), &["add", "."]);
    run_git(directory.path(), &["commit", "-m", "track derived state"]);

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
