use super::*;

pub(super) fn assert_managed_runtime_is_ignored_but_project_metadata_is_dirty(
    repository: &Path,
) {
    fs::create_dir_all(repository.join(".usdhub/cache")).unwrap();
    fs::create_dir_all(repository.join(".usdhub/recovery")).unwrap();
    fs::write(repository.join(".usdhub/cache/runtime.bin"), b"runtime").unwrap();
    fs::write(repository.join(".usdhub/recovery/recovery.bin"), b"recovery").unwrap();
    assert!(!usd_git::Repository::open(repository)
        .unwrap()
        .working_tree_status()
        .unwrap()
        .dirty);

    let project_metadata = repository.join("project.json");
    let original_project_metadata = fs::read(&project_metadata).unwrap();
    fs::write(&project_metadata, b"canonical Project metadata edit").unwrap();
    assert!(usd_git::Repository::open(repository)
        .unwrap()
        .working_tree_status()
        .unwrap()
        .dirty);
    fs::write(project_metadata, original_project_metadata).unwrap();
}
