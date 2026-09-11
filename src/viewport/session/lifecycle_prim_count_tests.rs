use std::{fs, path::{Path, PathBuf}};

use bevy::prelude::World;
use tempfile::tempdir;
use usd_model::HashDigest;

use crate::project::cache::SceneCacheStore;
use crate::project::cache_contract::{
    SCENE_CACHE_INDEX_SCHEMA_VERSION, SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
    SceneCacheDescriptorV3, SceneCacheIndex, SceneCacheState, SceneSpatialIndex,
};
use crate::project::storage::ProjectStorageLayout;
use crate::viewport::session::SceneDerivedMetadata;

use super::{
    CachePublication, SceneDerivedMetadataFreshness, publish_scene_cache_count, retry_allowed,
};

#[test]
fn prim_count_retries_have_a_terminal_bound() {
    assert!(retry_allowed(0));
    assert!(retry_allowed(2));
    assert!(!retry_allowed(3));
    assert!(!retry_allowed(u8::MAX));
}

#[test]
fn malformed_index_publication_is_retryable_lifecycle_failure() -> anyhow::Result<()> {
    exercise_malformed_cache_read_failure(true)
}

#[test]
fn malformed_spatial_publication_is_retryable_lifecycle_failure() -> anyhow::Result<()> {
    exercise_malformed_cache_read_failure(false)
}

#[test]
fn same_generation_merge_preserves_latest_cache_metadata() -> anyhow::Result<()> {
    let directory = tempdir().expect("Scene cache test directory");
    let scene_id = usd_project::SceneId::new_v4();
    let config_hash = HashDigest::new([21; HashDigest::BYTE_LEN]);
    let store = SceneCacheStore::new(directory.path());
    let expected = descriptor(scene_id, 21, config_hash);
    let index = empty_index(scene_id, 21);
    let spatial = empty_spatial(scene_id, 21);
    store
        .publish_generation(&expected, &index, &spatial)
        .expect("publish queued-inspection descriptor");
    let mut latest = expected.clone();
    latest.estimated_cpu_bytes = 91;
    latest.estimated_gpu_bytes = 37;
    store
        .publish_generation(&latest, &index, &spatial)
        .expect("publish same-generation cache evolution");

    let mut world = world_with_metadata(directory.path(), scene_id, expected);
    let freshness = world
        .resource::<SceneDerivedMetadata>()
        .freshness()
        .expect("current freshness");
    let result = publish_scene_cache_count(&mut world, &freshness, 8);
    let CachePublication::Published(published) = result else {
        panic!("same-generation cache evolution must merge, not be stale");
    };

    assert_eq!(published.estimated_cpu_bytes, 91);
    assert_eq!(published.estimated_gpu_bytes, 37);
    assert_eq!(published.prim_count, 8);
    assert_eq!(store.load_descriptor(scene_id)?.unwrap(), published);
    Ok::<(), anyhow::Error>(())
}

#[test]
fn newer_generation_rejection_leaves_lifecycle_metadata_unchanged() -> anyhow::Result<()> {
    let directory = tempdir().expect("Scene cache test directory");
    let scene_id = usd_project::SceneId::new_v4();
    let config_hash = HashDigest::new([22; HashDigest::BYTE_LEN]);
    let store = SceneCacheStore::new(directory.path());
    let expected = descriptor(scene_id, 22, config_hash);
    let index = empty_index(scene_id, 22);
    let spatial = empty_spatial(scene_id, 22);
    store
        .publish_generation(&expected, &index, &spatial)
        .expect("publish expected generation");
    store
        .advance_generation(scene_id, config_hash)
        .expect("advance newer generation");
    let mut world = world_with_metadata(directory.path(), scene_id, expected);
    let freshness = world
        .resource::<SceneDerivedMetadata>()
        .freshness()
        .expect("stale freshness");

    assert!(matches!(
        publish_scene_cache_count(&mut world, &freshness, 9),
        CachePublication::Rejected
    ));
    let metadata = world.resource::<SceneDerivedMetadata>();
    assert_eq!(metadata.cache_generation, Some(22));
    assert_eq!(metadata.prim_count, 0);
    assert!(!metadata.prim_count_ready);
    Ok::<(), anyhow::Error>(())
}

#[test]
fn descriptor_appearing_after_uncached_activation_reconciles_same_session() {
    let directory = tempdir().expect("Scene cache test directory");
    let scene_id = usd_project::SceneId::new_v4();
    let config_hash = HashDigest::new([23; HashDigest::BYTE_LEN]);
    let store = SceneCacheStore::new(directory.path());
    let current = descriptor(scene_id, 1, config_hash);
    store
        .publish_generation(
            &current,
            &empty_index(scene_id, 1),
            &empty_spatial(scene_id, 1),
        )
        .expect("publish concurrently appearing descriptor");

    let mut metadata = SceneDerivedMetadata::from_activation(
        directory.path().join("scene.usda"),
        23,
        Some((directory.path().to_path_buf(), scene_id)),
        None,
        None,
    );
    metadata.bind_session(23);
    let mut world = World::new();
    world.insert_resource(metadata);
    let first_freshness = world
        .resource::<SceneDerivedMetadata>()
        .freshness()
        .expect("uncached freshness");
    let first = publish_scene_cache_count(&mut world, &first_freshness, 6);
    let CachePublication::Retryable(refreshed) = first else {
        panic!("descriptor appearance must request same-session reconciliation");
    };
    super::reconcile_retryable_cache(&mut world, &refreshed);
    assert_eq!(world.resource::<SceneDerivedMetadata>().session_id, Some(23));
    assert_eq!(world.resource::<SceneDerivedMetadata>().cache_generation, Some(1));
    assert!(!world.resource::<SceneDerivedMetadata>().prim_count_ready);

    super::publish_projected_count(&mut world, 23, 6);
    assert!(world.resource::<SceneDerivedMetadata>().prim_count_ready);
}

#[test]
fn late_prior_session_result_cannot_mutate_current_lifecycle() {
    let directory = tempdir().expect("Scene cache test directory");
    let scene_id = usd_project::SceneId::new_v4();
    let config_hash = HashDigest::new([24; HashDigest::BYTE_LEN]);
    let store = SceneCacheStore::new(directory.path());
    let expected = descriptor(scene_id, 24, config_hash);
    store
        .publish_generation(
            &expected,
            &empty_index(scene_id, 24),
            &empty_spatial(scene_id, 24),
        )
        .expect("publish session descriptor");
    let mut world = world_with_metadata(directory.path(), scene_id, expected);
    world.resource_mut::<SceneDerivedMetadata>().bind_session(2);
    let mut stale = world
        .resource::<SceneDerivedMetadata>()
        .freshness()
        .expect("current freshness");
    stale.session_id = 1;
    assert!(matches!(
        publish_scene_cache_count(&mut world, &stale, 10),
        CachePublication::Rejected
    ));
    let metadata = world.resource::<SceneDerivedMetadata>();
    assert_eq!(metadata.session_id, Some(2));
    assert_eq!(metadata.prim_count, 0);
    assert!(!metadata.prim_count_ready);
}

#[test]
fn ordinary_non_project_activation_has_no_cache_write() {
    let mut world = World::new();
    let mut metadata = SceneDerivedMetadata::uncached(PathBuf::from("/tmp/ordinary.usda"));
    metadata.bind_session(25);
    world.insert_resource(metadata);
    let freshness = world
        .resource::<SceneDerivedMetadata>()
        .freshness()
        .expect("ordinary freshness");

    assert!(matches!(
        publish_scene_cache_count(&mut world, &freshness, 3),
        CachePublication::NotApplicable
    ));
    assert!(world
        .resource::<SceneDerivedMetadata>()
        .cache_project_root
        .is_none());
}

fn exercise_malformed_cache_read_failure(index_artifact: bool) -> anyhow::Result<()> {
    let directory = tempdir().expect("Scene cache test directory");
    let scene_id = usd_project::SceneId::new_v4();
    let config_hash = HashDigest::new([25; HashDigest::BYTE_LEN]);
    let store = SceneCacheStore::new(directory.path());
    let expected = descriptor(scene_id, 25, config_hash);
    let expected = store.publish_generation(
        &expected,
        &empty_index(scene_id, 25),
        &empty_spatial(scene_id, 25),
    )?;
    let layout = ProjectStorageLayout::new(directory.path());
    let artifact_path = if index_artifact {
        layout.scene_cache_index_path(scene_id)
    } else {
        layout.scene_cache_spatial_path(scene_id)
    };
    fs::write(artifact_path, b"not-json")?;

    let mut world = world_with_metadata(directory.path(), scene_id, expected.clone());
    let freshness = world
        .resource::<SceneDerivedMetadata>()
        .freshness()
        .expect("current freshness");
    let prior_session = freshness.clone();
    for attempt in 1..=super::MAX_PRIM_COUNT_ATTEMPTS {
        {
            let mut metadata = world.resource_mut::<SceneDerivedMetadata>();
            metadata.prim_count_pending = true;
            metadata.prim_count_ready = false;
            metadata.prim_count_attempts = attempt;
            metadata.prim_count_terminal = false;
            metadata.prim_count_error = None;
        }
        let publication = super::publish_scene_cache_count(&mut world, &freshness, 99);
        assert!(matches!(publication, CachePublication::Failed(_)));
        super::record_retryable_failure(&mut world, "cache artifact read failed".to_owned());

        let metadata = world.resource::<SceneDerivedMetadata>();
        assert!(!metadata.prim_count_pending);
        assert!(!metadata.prim_count_ready);
        assert_eq!(metadata.prim_count_terminal, attempt == super::MAX_PRIM_COUNT_ATTEMPTS);
        assert!(metadata
            .prim_count_error
            .as_deref()
            .is_some_and(|error| error.contains("cache artifact read failed")));
        assert_eq!(metadata.cache_descriptor.as_ref(), Some(&expected));
        assert_eq!(metadata.cache_generation, Some(expected.generation));
        assert_eq!(store.load_descriptor(scene_id)?.as_ref(), Some(&expected));
    }

    world.resource_mut::<SceneDerivedMetadata>().bind_session(24);
    let metadata = world.resource::<SceneDerivedMetadata>();
    assert_eq!(metadata.session_id, Some(24));
    assert!(!metadata.prim_count_pending);
    assert!(!metadata.prim_count_ready);
    assert_eq!(metadata.prim_count_attempts, 0);
    assert!(!metadata.prim_count_terminal);
    assert!(metadata.prim_count_error.is_none());
    assert!(matches!(
        super::publish_scene_cache_count(&mut world, &prior_session, 101),
        CachePublication::Rejected
    ));
    assert_eq!(world.resource::<SceneDerivedMetadata>().session_id, Some(24));
    assert_eq!(world.resource::<SceneDerivedMetadata>().cache_descriptor.as_ref(), Some(&expected));
    assert_eq!(store.load_descriptor(scene_id)?.as_ref(), Some(&expected));
    Ok(())
}

fn descriptor(
    scene_id: usd_project::SceneId,
    generation: u64,
    config_hash: HashDigest,
) -> SceneCacheDescriptorV3 {
    let mut descriptor = SceneCacheDescriptorV3::invalidated(scene_id, generation, config_hash);
    descriptor.state = SceneCacheState::Partial;
    descriptor
}

fn empty_index(scene_id: usd_project::SceneId, generation: u64) -> SceneCacheIndex {
    SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation,
        entries: Vec::new(),
    }
}

fn empty_spatial(scene_id: usd_project::SceneId, generation: u64) -> SceneSpatialIndex {
    SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id,
        generation,
        entries: Vec::new(),
    }
}

fn world_with_metadata(
    project_root: &Path,
    scene_id: usd_project::SceneId,
    descriptor: SceneCacheDescriptorV3,
) -> World {
    let mut metadata = SceneDerivedMetadata::from_activation(
        project_root.join("scene.usda"),
        descriptor.generation,
        Some((project_root.to_path_buf(), scene_id)),
        None,
        Some(descriptor),
    );
    metadata.bind_session(23);
    let mut world = World::new();
    world.insert_resource(metadata);
    world
}
