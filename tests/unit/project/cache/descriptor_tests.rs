use tempfile::tempdir;
use usd_model::HashDigest;

use super::*;

fn make_identity(id: &str) -> ProjectCacheIdentity {
    ProjectCacheIdentity {
        target: ProjectCacheTarget::Scene { id: id.to_owned() },
        target_content_hash: HashDigest::new([6; HashDigest::BYTE_LEN]),
        profile: RuntimeProfile::NativeMedium,
        config_hash: HashDigest::new([7; HashDigest::BYTE_LEN]),
    }
}

#[test]
fn descriptor_round_trip_is_atomic_and_target_specific() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let store = ProjectCacheStore::new(directory.path());
    let identity = make_identity("scene-a");
    let descriptor =
        ProjectCacheDescriptor::new(identity.clone(), ProjectCacheState::Partial, None)?;

    let path = store.publish(&descriptor)?;
    assert!(path.is_file());
    assert_eq!(store.load(&identity)?, Some(descriptor));
    assert_eq!(fs::read_dir(path.parent().unwrap())?.count(), 1);

    let other = make_identity("scene-b");
    assert_ne!(
        store.descriptor_path(&identity)?,
        store.descriptor_path(&other)?
    );
    Ok(())
}

#[test]
fn corrupt_descriptor_is_rejected_without_source_fallback_data() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let store = ProjectCacheStore::new(directory.path());
    let identity = make_identity("scene-a");
    let path = store.descriptor_path(&identity)?;
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(&path, b"not-json")?;

    assert!(store.load(&identity).is_err());
    Ok(())
}

#[test]
fn stale_descriptor_schema_is_rejected_and_config_identity_is_a_miss() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let store = ProjectCacheStore::new(directory.path());
    let identity = make_identity("scene-a");
    let descriptor =
        ProjectCacheDescriptor::new(identity.clone(), ProjectCacheState::Partial, None)?;
    let path = store.publish(&descriptor)?;

    let mut stale = serde_json::to_value(&descriptor)?;
    stale["schema_version"] = serde_json::json!(999);
    fs::write(&path, serde_json::to_vec(&stale)?)?;
    assert!(store.load(&identity).is_err());

    store.publish(&descriptor)?;
    let different_config = ProjectCacheIdentity {
        config_hash: HashDigest::new([8; HashDigest::BYTE_LEN]),
        ..identity.clone()
    };
    assert!(store.load(&different_config)?.is_none());
    Ok(())
}

#[test]
fn descriptors_reuse_when_target_content_is_unchanged() -> Result<()> {
    let directory = tempdir()?;
    let store = ProjectCacheStore::new(directory.path());
    let target = ProjectCacheTarget::ProjectRoot;
    let identity_a = ProjectCacheIdentity {
        target: target.clone(),
        target_content_hash: HashDigest::new([9; HashDigest::BYTE_LEN]),
        profile: RuntimeProfile::NativeMedium,
        config_hash: HashDigest::new([7; HashDigest::BYTE_LEN]),
    };
    let identity_b = ProjectCacheIdentity {
        ..identity_a.clone()
    };
    store.publish(&ProjectCacheDescriptor::new(
        identity_a.clone(),
        ProjectCacheState::Partial,
        None,
    )?)?;

    assert!(store.load(&identity_a)?.is_some());
    assert!(store.load(&identity_b)?.is_some());
    assert_eq!(
        store.descriptor_path(&identity_a)?,
        store.descriptor_path(&identity_b)?
    );
    Ok(())
}

#[test]
fn removing_a_target_drops_descriptors_but_keeps_the_object_store() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let store = ProjectCacheStore::new(directory.path());
    let target = ProjectCacheTarget::Scene {
        id: "scene-a".to_owned(),
    };
    let identity = ProjectCacheIdentity {
        target_content_hash: HashDigest::new([1; HashDigest::BYTE_LEN]),
        target: target.clone(),
        profile: RuntimeProfile::NativeMedium,
        config_hash: HashDigest::new([2; HashDigest::BYTE_LEN]),
    };
    store.publish(&ProjectCacheDescriptor::new(
        identity.clone(),
        ProjectCacheState::Partial,
        None,
    )?)?;
    assert_eq!(store.remove_target(&target)?, 1);
    assert!(store.load(&identity)?.is_none());
    Ok(())
}
