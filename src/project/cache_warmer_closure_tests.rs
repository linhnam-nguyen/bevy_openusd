use std::fs;
use std::time::Duration;

use anyhow::Result;
use tempfile::tempdir;
use usd_project::{
    ModelManifestEntry, ModelSourceKind, ProjectId, ProjectManifestV1, ProjectRoot, SceneId,
    StorageKey,
};

use super::*;
use crate::project::catalog::manifest_store::ManifestStore;

#[test]
fn imported_scene_dependency_closure_changes_only_composed_targets() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_a = SceneId::new_v4();
    let scene_b = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Imported Scene Closure",
        ProjectRoot::Scene(scene_a),
        vec![
            usd_project::SceneManifestEntry {
                id: scene_a,
                storage_key: StorageKey::new("scene-a")?,
                display_name: "Scene A".to_owned(),
            },
            usd_project::SceneManifestEntry {
                id: scene_b,
                storage_key: StorageKey::new("scene-b")?,
                display_name: "Scene B".to_owned(),
            },
        ],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    let wrapper_a =
        crate::project::scene::authoring::author_scene_atomic(directory.path(), scene_a)?;
    crate::project::scene::authoring::author_scene_atomic(directory.path(), scene_b)?;

    let imports = directory
        .path()
        .join("imports/scenes")
        .join(scene_a.to_string());
    fs::create_dir_all(&imports)?;
    let source_a = imports.join("source.usda");
    fs::write(&source_a, b"imported-source-v1")?;
    let imports_b = directory
        .path()
        .join("imports/scenes")
        .join(scene_b.to_string());
    fs::create_dir_all(&imports_b)?;
    let source_b = imports_b.join("source.usda");
    fs::write(&source_b, b"sibling-source-v1")?;

    let target_a = ProjectCacheTarget::Scene {
        id: scene_a.to_string(),
    };
    let target_b = ProjectCacheTarget::Scene {
        id: scene_b.to_string(),
    };
    let root = ProjectCacheTarget::ProjectRoot;
    let identity_a_v1 = ProjectCacheIdentity::for_project(
        directory.path(),
        target_a.clone(),
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;
    let root_v1 = ProjectCacheIdentity::for_project(
        directory.path(),
        root.clone(),
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;
    let identity_b_v1 = ProjectCacheIdentity::for_project(
        directory.path(),
        target_b.clone(),
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;
    let wrapper_bytes = fs::read(&wrapper_a)?;

    fs::write(&source_a, b"imported-source-v2")?;
    let identity_a_v2 = ProjectCacheIdentity::for_project(
        directory.path(),
        target_a.clone(),
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;
    let root_v2 = ProjectCacheIdentity::for_project(
        directory.path(),
        root.clone(),
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;
    assert_ne!(identity_a_v1, identity_a_v2);
    assert_ne!(root_v1, root_v2);
    assert_eq!(fs::read(&wrapper_a)?, wrapper_bytes);

    fs::write(&source_a, b"imported-source-v1")?;
    let identity_a_restored = ProjectCacheIdentity::for_project(
        directory.path(),
        target_a.clone(),
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;
    assert_eq!(identity_a_v1, identity_a_restored);

    fs::write(&source_b, b"sibling-source-v2")?;
    let identity_a_after_sibling_edit = ProjectCacheIdentity::for_project(
        directory.path(),
        target_a.clone(),
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;
    let root_after_sibling_edit = ProjectCacheIdentity::for_project(
        directory.path(),
        root,
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;
    assert_eq!(identity_a_restored, identity_a_after_sibling_edit);
    assert_ne!(root_v2, root_after_sibling_edit);
    assert_eq!(identity_b_v1.target, target_b);
    Ok(())
}

#[test]
fn target_cache_identity_changes_when_authoritative_display_name_changes() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_id = SceneId::new_v4();
    let project_id = ProjectId::new_v4();
    let manifest = ProjectManifestV1::new(
        project_id,
        "Identity Project",
        ProjectRoot::Scene(scene_id),
        vec![usd_project::SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("architecture")?,
            display_name: "Architecture".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    crate::project::scene::authoring::author_scene_atomic(directory.path(), scene_id)?;

    let target = ProjectCacheTarget::Scene {
        id: scene_id.to_string(),
    };
    let before = ProjectCacheIdentity::for_project(
        directory.path(),
        target.clone(),
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;

    let mut renamed = manifest;
    renamed.scenes[0].display_name = "Architecture Revised".to_owned();
    ManifestStore::write_manifest_atomic(directory.path(), &renamed)?;
    let after = ProjectCacheIdentity::for_project(
        directory.path(),
        target,
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;

    assert_ne!(
        before.target_content_hash, after.target_content_hash,
        "stale runtime labels must not survive a canonical name change"
    );
    Ok(())
}

#[test]
fn activation_preparation_returns_fallback_when_runtime_warm_fails() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let model_id = usd_project::ModelId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Failed Runtime Warm",
        ProjectRoot::Model(model_id),
        Vec::new(),
        vec![ModelManifestEntry {
            id: model_id,
            source_kind: ModelSourceKind::Usd,
            storage_key: StorageKey::new("model")?,
            display_name: "Model".to_owned(),
        }],
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    let wrapper = crate::project::model_wrapper::model_wrapper_path(directory.path(), model_id);
    fs::create_dir_all(wrapper.parent().expect("Model wrapper directory"))?;
    fs::write(&wrapper, b"#usda 1.0\n(this is not a valid USD layer")?;

    let target = ProjectCacheTarget::Model {
        id: model_id.to_string(),
    };
    let queue = ProjectCacheWarmQueue::default();
    assert!(queue.enqueue(directory.path(), target.clone()));
    assert!(queue.wait_for_project_idle(directory.path(), Duration::from_secs(2)));
    assert_eq!(
        queue.prepare_for_activation(directory.path(), target.clone()),
        ProjectCachePreparation::FallbackRequired
    );
    let identity = ProjectCacheIdentity::for_project(
        directory.path(),
        target,
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;
    let descriptor = ProjectCacheStore::new(directory.path())
        .load(&identity)?
        .expect("failed warm publishes a descriptor");
    assert_eq!(descriptor.state, ProjectCacheState::FallbackRequired);
    Ok(())
}

#[test]
fn stale_scene_generation_is_rejected_before_strong_identity_or_stage_open() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_id = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Early Stale Guard",
        ProjectRoot::Scene(scene_id),
        vec![usd_project::SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("scene")?,
            display_name: "Scene".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    let config_hash = crate::project::cache_hydration::default_project_cache_config_hash();
    let store = SceneCacheStore::new(directory.path());
    let stale_generation = store.advance_generation(scene_id, config_hash)?;
    assert_eq!(store.advance_generation(scene_id, config_hash)?, stale_generation + 1);

    let target = WarmTarget {
        target: ProjectCacheTarget::Scene { id: scene_id.to_string() },
        scene_generation: Some(stale_generation),
        build_generation: 1,
    };
    warm_target(directory.path(), &target)?;
    assert!(!crate::project::scene::authoring::scene_path(directory.path(), scene_id).exists());
    assert_eq!(store.load_descriptor(scene_id)?.unwrap().generation, stale_generation + 1);
    Ok(())
}

#[test]
fn stale_managed_scene_generation_cannot_publish() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_id = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Generation Guard",
        ProjectRoot::Scene(scene_id),
        vec![usd_project::SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("scene")?,
            display_name: "Scene".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    crate::project::scene::authoring::author_scene_atomic(directory.path(), scene_id)?;
    let config_hash = crate::project::cache_hydration::default_project_cache_config_hash();
    let store = SceneCacheStore::new(directory.path());
    let generation = store.advance_generation(scene_id, config_hash)?;
    let identity = ProjectCacheIdentity::for_project(
        directory.path(),
        ProjectCacheTarget::Scene { id: scene_id.to_string() },
        RuntimeProfile::NativeMedium,
        config_hash,
    )?;
    let mut stale = SceneCacheDescriptorV3::invalidated(scene_id, generation, config_hash);
    stale.source_content_hash = Some(identity.target_content_hash);
    stale.state = SceneCacheState::Partial;
    assert_eq!(store.advance_generation(scene_id, config_hash)?, generation + 1);

    assert!(crate::project::cache_warm_runtime::build_and_publish_managed_scene_cache_generation(
        directory.path(),
        &stale,
    )?
    .is_none());
    let current = store.load_descriptor(scene_id)?.expect("newer invalidation survives");
    assert_eq!(current.generation, generation + 1);
    assert_eq!(current.state, SceneCacheState::Building);
    Ok(())
}

#[test]
fn scene_deletion_removes_v3_cache_and_rebuilds_project_lookup() -> Result<()> {
    use crate::project::blob_store::BlobStore;

    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_a = SceneId::new_v4();
    let scene_b = SceneId::new_v4();
    let mut manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Deletion Cache",
        ProjectRoot::Scene(scene_a),
        vec![
            usd_project::SceneManifestEntry { id: scene_a, storage_key: StorageKey::new("a")?, display_name: "A".to_owned() },
            usd_project::SceneManifestEntry { id: scene_b, storage_key: StorageKey::new("b")?, display_name: "B".to_owned() },
        ],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    let config_hash = crate::project::cache_hydration::default_project_cache_config_hash();
    let store = SceneCacheStore::new(directory.path());
    store.publish_descriptor(&SceneCacheDescriptorV3::invalidated(scene_a, 1, config_hash))?;
    store.publish_descriptor(&SceneCacheDescriptorV3::invalidated(scene_b, 1, config_hash))?;
    store.object_store(scene_a)?.put(b"keep")?;
    store.object_store(scene_b)?.put(b"delete")?;
    let layout = crate::project::storage::ProjectStorageLayout::new(directory.path());
    fs::write(layout.scene_cache_index_path(scene_b), b"deleted-index")?;
    fs::write(layout.scene_cache_spatial_path(scene_b), b"deleted-spatial")?;
    fs::write(layout.project_cache_index_path(), scene_b.to_string())?;

    manifest.scenes.retain(|scene| scene.id != scene_b);
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    let queue = ProjectCacheWarmQueue::default();
    assert!(queue.remove_target_descriptors(
        directory.path(),
        &ProjectCacheTarget::Scene { id: scene_b.to_string() },
    ));
    assert!(!layout.scene_cache_dir(scene_b).exists());
    assert!(!layout.scene_cache_objects_dir(scene_b).exists());
    assert!(layout.scene_cache_dir(scene_a).exists());
    assert!(!fs::read_to_string(layout.project_cache_index_path())?.contains(&scene_b.to_string()));
    Ok(())
}
#[test]
fn scene_deletion_serializes_with_scene_owned_persistence_guard() -> Result<()> {
    use crate::project::blob_store::BlobStore;

    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_id = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Deletion Guard",
        ProjectRoot::Scene(scene_id),
        vec![usd_project::SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("scene")?,
            display_name: "Scene".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    crate::project::scene::authoring::author_scene_atomic(directory.path(), scene_id)?;
    let store = SceneCacheStore::new(directory.path());
    store.advance_generation(scene_id, crate::project::cache_hydration::default_project_cache_config_hash())?;
    store.object_store(scene_id)?.put(b"owned-before-delete")?;

    let scene_lock = scene_lifecycle_lock(directory.path(), scene_id);
    let guard = scene_lock.lock().expect("Scene lifecycle test lock is not poisoned");
    let queue = ProjectCacheWarmQueue::default();
    let root = directory.path().to_path_buf();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let removed = queue.remove_target_descriptors(
            &root,
            &ProjectCacheTarget::Scene { id: scene_id.to_string() },
        );
        done_tx.send(removed).unwrap();
    });
    started_rx.recv().unwrap();
    assert!(done_rx.recv_timeout(Duration::from_millis(25)).is_err());
    drop(guard);
    assert!(done_rx.recv_timeout(Duration::from_secs(1)).unwrap());
    worker.join().unwrap();
    assert!(store.load_descriptor(scene_id)?.is_none());
    assert!(!crate::project::storage::ProjectStorageLayout::new(directory.path()).scene_cache_objects_dir(scene_id).exists());
    Ok(())
}
