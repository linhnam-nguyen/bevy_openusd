use std::fs;

use super::*;
use tempfile::tempdir;

#[test]
fn unborn_repository_uses_working_tree_fingerprint() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    fs::write(directory.path().join("scene.usda"), b"scene")?;

    let first = inspect_git_baseline(directory.path())?;
    assert!(first.head.is_none());
    assert!(first.dirty);
    assert!(matches!(
        &first.source,
        ProjectCacheSourceStamp::WorkingTree { .. }
    ));

    fs::write(directory.path().join("scene.usda"), b"changed")?;
    let second = inspect_git_baseline(directory.path())?;
    assert_ne!(first.source, second.source);
    Ok(())
}

#[test]
fn disposable_cache_and_recovery_do_not_change_working_fingerprint() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    fs::write(directory.path().join("scene.usda"), b"scene")?;
    let first = fingerprint_project(directory.path())?;

    fs::create_dir_all(directory.path().join(".usdhub/cache/objects"))?;
    fs::create_dir_all(directory.path().join(".usdhub/recovery"))?;
    fs::write(
        directory.path().join(".usdhub/cache/objects/object.blob"),
        b"derived",
    )?;
    fs::write(
        directory.path().join(".usdhub/recovery/pending"),
        b"derived",
    )?;

    assert_eq!(fingerprint_project(directory.path())?, first);
    Ok(())
}

#[test]
fn clean_head_uses_commit_oid() {
    assert_eq!(
        source_stamp(false, Some("abc123"), "working".to_owned()),
        ProjectCacheSourceStamp::GitCommit {
            oid: "abc123".to_owned()
        }
    );
}
