use std::fs;

use anyhow::Result;
use tempfile::tempdir;
use usd_project::{ProjectId, ProjectManifestV1, ProjectRoot, SceneId, StorageKey};

use super::super::{
    cache_hydration::default_project_cache_config_hash, catalog::manifest_store::ManifestStore,
    storage::ProjectStorageLayout,
};
use super::{
    ProjectCacheTarget, ProjectCacheWarmQueue, SceneCacheDescriptorV3, SceneCacheState,
    SceneCacheStore, enqueue_project_targets_fail_closed,
};

#[test]
fn early_scene_recovery_failure_does_not_short_circuit_later_scene_invalidation() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let first_scene = SceneId::new_v4();
    let later_scene = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Exhaustive Scene Recovery",
        ProjectRoot::Empty,
        vec![
            usd_project::SceneManifestEntry {
                id: first_scene,
                storage_key: StorageKey::new("first")?,
                display_name: "First".to_owned(),
            },
            usd_project::SceneManifestEntry {
                id: later_scene,
                storage_key: StorageKey::new("later")?,
                display_name: "Later".to_owned(),
            },
        ],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;

    let layout = ProjectStorageLayout::new(directory.path());
    fs::create_dir_all(layout.scene_cache_descriptor_path(first_scene))?;
    let queue = ProjectCacheWarmQueue::default();
    queue.shutdown_without_waiting();

    assert!(enqueue_project_targets_fail_closed(
        &queue,
        directory.path()
    ));
    assert!(!layout.scene_cache_dir(first_scene).exists());
    let later = SceneCacheStore::new(directory.path())
        .load_descriptor(later_scene)?
        .expect("later Scene boundary was not attempted");
    assert_eq!(later.state, SceneCacheState::Building);
    assert!(later.generation >= 2);
    Ok(())
}

#[test]
fn generation_overflow_uses_scene_cleanup_fallback() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_id = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Overflow Scene Recovery",
        ProjectRoot::Empty,
        vec![usd_project::SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("overflow")?,
            display_name: "Overflow".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;

    let store = SceneCacheStore::new(directory.path());
    let config_hash = default_project_cache_config_hash();
    store.publish_descriptor(&SceneCacheDescriptorV3::invalidated(
        scene_id,
        u64::MAX,
        config_hash,
    ))?;
    let queue = ProjectCacheWarmQueue::default();
    queue.shutdown_without_waiting();

    assert!(enqueue_project_targets_fail_closed(
        &queue,
        directory.path()
    ));
    assert!(store.load_descriptor(scene_id)?.is_none());
    assert!(
        !ProjectStorageLayout::new(directory.path())
            .scene_cache_dir(scene_id)
            .exists()
    );
    Ok(())
}

#[test]
fn managed_mutation_generation_overflow_removes_old_scene_cache_before_return() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_id = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Managed Mutation Overflow",
        ProjectRoot::Empty,
        vec![usd_project::SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("managed-overflow")?,
            display_name: "Managed Overflow".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;

    let store = SceneCacheStore::new(directory.path());
    store.publish_descriptor(&SceneCacheDescriptorV3::invalidated(
        scene_id,
        u64::MAX,
        default_project_cache_config_hash(),
    ))?;
    let queue = ProjectCacheWarmQueue::default();
    queue.shutdown_without_waiting();

    assert!(!queue.enqueue_targets_for_mutation(
        directory.path(),
        vec![ProjectCacheTarget::Scene {
            id: scene_id.to_string()
        }],
    )?);
    assert!(store.load_descriptor(scene_id)?.is_none());
    assert!(
        !ProjectStorageLayout::new(directory.path())
            .scene_cache_dir(scene_id)
            .exists()
    );
    Ok(())
}

#[test]
fn corrupt_descriptor_uses_scene_cleanup_fallback() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_id = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Corrupt Scene Recovery",
        ProjectRoot::Empty,
        vec![usd_project::SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("corrupt")?,
            display_name: "Corrupt".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;

    let layout = ProjectStorageLayout::new(directory.path());
    fs::create_dir_all(layout.scene_cache_dir(scene_id))?;
    fs::write(layout.scene_cache_descriptor_path(scene_id), b"not-json")?;
    let queue = ProjectCacheWarmQueue::default();
    queue.shutdown_without_waiting();

    assert!(enqueue_project_targets_fail_closed(
        &queue,
        directory.path()
    ));
    assert!(!layout.scene_cache_dir(scene_id).exists());
    Ok(())
}
