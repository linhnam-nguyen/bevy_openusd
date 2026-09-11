use std::fs;

use tempfile::tempdir;
use usd_model::HashDigest;
use usd_project::SceneId;

use crate::project::cache::SceneCacheStore;
use crate::project::cache_contract::{
    SCENE_CACHE_INDEX_SCHEMA_VERSION, SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
    SceneCacheDescriptorV3, SceneCacheIndex, SceneCacheState, SceneSpatialIndex,
};
use crate::project::storage::ProjectStorageLayout;

use super::{PrimCountPublication, publish_prim_count_if_current};

#[test]
fn prim_count_publication_reopens_partial_scene_without_inspection() {
    let directory = tempdir().expect("Scene cache test directory");
    let scene_id = SceneId::new_v4();
    let config_hash = HashDigest::new([7; HashDigest::BYTE_LEN]);
    let descriptor = SceneCacheDescriptorV3 {
        state: SceneCacheState::Partial,
        prim_count: 2,
        ..SceneCacheDescriptorV3::invalidated(scene_id, 13, config_hash)
    };
    let index = empty_index(scene_id, 13);
    let spatial = empty_spatial(scene_id, 13);
    let store = SceneCacheStore::new(directory.path());
    let expected = store
        .publish_generation(&descriptor, &index, &spatial)
        .expect("publish Partial descriptor");
    let result = publish_prim_count_if_current(
        &store,
        scene_id,
        Some(&expected),
        config_hash,
        5,
    )
    .expect("publish background prim count");
    let PrimCountPublication::Published(published) = result else {
        panic!("fresh Scene generation remains current");
    };

    assert_eq!(published.prim_count, 5);
    assert!(published.prim_count_ready);
    let reopened = store
        .load_activation(scene_id)
        .expect("reopen Scene cache")
        .expect("Partial metadata is reusable");
    assert_eq!(reopened.descriptor.prim_count, 5);
    assert!(reopened.descriptor.prim_count_ready);
}

#[test]
fn prim_count_publication_rejects_stale_scene_generation() -> anyhow::Result<()> {
    let directory = tempdir().expect("Scene cache test directory");
    let scene_id = SceneId::new_v4();
    let config_hash = HashDigest::new([8; HashDigest::BYTE_LEN]);
    let store = SceneCacheStore::new(directory.path());
    let expected = store
        .publish_generation(
            &SceneCacheDescriptorV3::invalidated(scene_id, 14, config_hash),
            &empty_index(scene_id, 14),
            &empty_spatial(scene_id, 14),
        )
        .expect("publish expected generation");
    store
        .advance_generation(scene_id, config_hash)
        .expect("advance Scene generation");

    assert!(matches!(
        publish_prim_count_if_current(&store, scene_id, Some(&expected), config_hash, 6)?,
        PrimCountPublication::Stale
    ));
    assert_eq!(
        store.load_descriptor(scene_id)?.map(|value| value.generation),
        Some(15)
    );
    Ok(())
}

#[test]
fn uncached_project_scene_owner_gets_reusable_prim_count_metadata() -> anyhow::Result<()> {
    let directory = tempdir().expect("Scene cache test directory");
    let scene_id = SceneId::new_v4();
    let store = SceneCacheStore::new(directory.path());
    let config_hash = HashDigest::new([9; HashDigest::BYTE_LEN]);
    let result = publish_prim_count_if_current(&store, scene_id, None, config_hash, 4)?;
    let PrimCountPublication::Published(published) = result else {
        panic!("create Scene-owned descriptor");
    };

    assert_eq!(published.scene_id, scene_id);
    assert_eq!(published.generation, 1);
    assert_eq!(published.prim_count, 4);
    assert!(published.prim_count_ready);
    assert!(store.load_activation(scene_id)?.is_some());
    Ok(())
}

#[test]
fn same_generation_updates_merge_under_scene_lock() -> anyhow::Result<()> {
    let directory = tempdir().expect("Scene cache test directory");
    let scene_id = SceneId::new_v4();
    let config_hash = HashDigest::new([10; HashDigest::BYTE_LEN]);
    let store = SceneCacheStore::new(directory.path());
    let descriptor = SceneCacheDescriptorV3 {
        state: SceneCacheState::Partial,
        ..SceneCacheDescriptorV3::invalidated(scene_id, 17, config_hash)
    };
    let index = empty_index(scene_id, 17);
    let spatial = empty_spatial(scene_id, 17);
    let expected = store
        .publish_generation(&descriptor, &index, &spatial)
        .expect("publish same-generation baseline");
    let first = publish_prim_count_if_current(&store, scene_id, Some(&expected), config_hash, 4)?;
    assert!(matches!(first, PrimCountPublication::Published(_)));
    let second = publish_prim_count_if_current(&store, scene_id, Some(&expected), config_hash, 5)?;
    assert!(matches!(second, PrimCountPublication::Published(_)));
    assert_eq!(store.load_descriptor(scene_id)?.unwrap().prim_count, 5);
    Ok(())
}

#[test]
fn index_read_failure_does_not_mutate_current_scene_cache() -> anyhow::Result<()> {
    let directory = tempdir().expect("Scene cache test directory");
    let scene_id = SceneId::new_v4();
    let config_hash = HashDigest::new([11; HashDigest::BYTE_LEN]);
    let store = SceneCacheStore::new(directory.path());
    let descriptor = SceneCacheDescriptorV3 {
        state: SceneCacheState::Partial,
        ..SceneCacheDescriptorV3::invalidated(scene_id, 18, config_hash)
    };
    let expected = store
        .publish_generation(
            &descriptor,
            &empty_index(scene_id, 18),
            &empty_spatial(scene_id, 18),
        )
        .expect("publish index failure baseline");
    fs::write(
        ProjectStorageLayout::new(directory.path()).scene_cache_index_path(scene_id),
        b"not-json",
    )?;
    assert!(
        publish_prim_count_if_current(&store, scene_id, Some(&expected), config_hash, 6)
            .is_err()
    );
    assert_eq!(store.load_descriptor(scene_id)?.unwrap(), expected);
    Ok(())
}

#[test]
fn spatial_read_failure_does_not_mutate_current_scene_cache() -> anyhow::Result<()> {
    let directory = tempdir().expect("Scene cache test directory");
    let scene_id = SceneId::new_v4();
    let config_hash = HashDigest::new([12; HashDigest::BYTE_LEN]);
    let store = SceneCacheStore::new(directory.path());
    let descriptor = SceneCacheDescriptorV3 {
        state: SceneCacheState::Partial,
        ..SceneCacheDescriptorV3::invalidated(scene_id, 19, config_hash)
    };
    let expected = store
        .publish_generation(
            &descriptor,
            &empty_index(scene_id, 19),
            &empty_spatial(scene_id, 19),
        )
        .expect("publish spatial failure baseline");
    fs::write(
        ProjectStorageLayout::new(directory.path()).scene_cache_spatial_path(scene_id),
        b"not-json",
    )?;
    assert!(
        publish_prim_count_if_current(&store, scene_id, Some(&expected), config_hash, 7)
            .is_err()
    );
    assert_eq!(store.load_descriptor(scene_id)?.unwrap(), expected);
    Ok(())
}

fn empty_index(scene_id: SceneId, generation: u64) -> SceneCacheIndex {
    SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation,
        entries: Vec::new(),
    }
}

fn empty_spatial(scene_id: SceneId, generation: u64) -> SceneSpatialIndex {
    SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id,
        generation,
        entries: Vec::new(),
    }
}
