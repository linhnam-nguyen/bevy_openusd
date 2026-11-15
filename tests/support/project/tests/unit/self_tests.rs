use super::*;

#[test]
fn test_configured_initial_branch() {
    // Isolate child-process Git configuration; do not modify process-wide settings.
    let directory = tempfile::tempdir().unwrap();
    let empty_config = directory.path().join("empty.gitconfig");
    fs::write(&empty_config, "").unwrap();
    let isolated_git = || {
        let mut command = Command::new("git");
        command
            .current_dir(directory.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &empty_config)
            .env_remove("GIT_CONFIG")
            .env_remove("GIT_CONFIG_COUNT")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE");
        command
    };
    assert_eq!(read_initial_branch(&mut isolated_git()), "main");
    let mut custom = isolated_git();
    custom.args(["-c", "init.defaultBranch=release"]);
    assert_eq!(read_initial_branch(&mut custom), "release");
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
    assert_eq!(
        content,
        include_bytes!("../../../../fixtures/usd/identity_original.usda").as_slice()
    );
}
