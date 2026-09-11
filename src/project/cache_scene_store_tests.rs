use super::*;
use crate::project::cache_contract::{
    CachedTransform, SCENE_CACHE_INDEX_SCHEMA_VERSION, SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
    SceneCacheAddress, SceneCacheEntry, SceneCacheEntryKind, SceneCacheOccurrence,
};
use tempfile::tempdir;

#[test]
fn partial_scene_activation_reads_descriptor_and_index_before_stage_open() {
    let directory = tempdir().expect("Scene cache test directory");
    let scene_id = SceneId::new_v4();
    let config_hash = HashDigest::new([1; HashDigest::BYTE_LEN]);
    let mut descriptor = SceneCacheDescriptorV3::invalidated(scene_id, 7, config_hash);
    descriptor.state = SceneCacheState::Partial;
    descriptor.source_content_hash = Some(HashDigest::new([2; HashDigest::BYTE_LEN]));
    let index = SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 7,
        entries: Vec::new(),
    };
    let spatial = SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 7,
        entries: Vec::new(),
    };
    let store = SceneCacheStore::new(directory.path());
    let published = store
        .publish_generation(&descriptor, &index, &spatial)
        .expect("publish Partial Scene cache");

    let activation = store
        .load_activation(scene_id)
        .expect("load Scene activation metadata")
        .expect("Partial Scene cache is usable");
    assert_eq!(activation.descriptor, published);
    assert_eq!(activation.index, index);

    descriptor.state = SceneCacheState::Building;
    store
        .publish_descriptor(&descriptor)
        .expect("publish Building descriptor");
    assert!(
        store
            .load_activation(scene_id)
            .expect("probe Building Scene cache")
            .is_none()
    );
}

#[test]
fn managed_generation_cas_rejects_same_generation_descriptor_mutation() {
    let directory = tempdir().expect("Scene cache test directory");
    let scene_id = SceneId::new_v4();
    let config_hash = HashDigest::new([3; HashDigest::BYTE_LEN]);
    let descriptor = SceneCacheDescriptorV3::invalidated(scene_id, 11, config_hash);
    let index = SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 11,
        entries: Vec::new(),
    };
    let spatial = SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 11,
        entries: Vec::new(),
    };
    let store = SceneCacheStore::new(directory.path());
    let published = store
        .publish_generation(&descriptor, &index, &spatial)
        .expect("publish expected Scene generation");

    let mut stale_expected = published.clone();
    stale_expected.config_hash = HashDigest::new([4; HashDigest::BYTE_LEN]);
    stale_expected.source_content_hash = Some(HashDigest::new([5; HashDigest::BYTE_LEN]));
    stale_expected.state = SceneCacheState::Ready;
    assert!(
        store
            .publish_generation_if_current(&stale_expected, &published, &index, &spatial)
            .expect("compare descriptor")
            .is_none()
    );
}

#[test]
fn concurrent_distinct_payload_cas_loser_preserves_winner_for_retry() {
    let directory = tempdir().expect("Scene cache test directory");
    let scene_id = SceneId::new_v4();
    let descriptor = SceneCacheDescriptorV3::invalidated(
        scene_id,
        12,
        HashDigest::new([6; HashDigest::BYTE_LEN]),
    );
    let empty = SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 12,
        entries: Vec::new(),
    };
    let spatial = SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 12,
        entries: Vec::new(),
    };
    let store = SceneCacheStore::new(directory.path());
    let expected = store
        .publish_generation(&descriptor, &empty, &spatial)
        .expect("publish expected generation");
    let winner_index = SceneCacheIndex {
        entries: vec![owned_entry(scene_id, "/SceneRoot/Winner")],
        ..empty.clone()
    };
    let winner_descriptor = store
        .publish_generation_if_current(&expected, &expected, &winner_index, &spatial)
        .expect("publish first distinct payload")
        .expect("first payload wins CAS");

    let loser_index = SceneCacheIndex {
        entries: vec![owned_entry(scene_id, "/SceneRoot/Loser")],
        ..empty
    };
    assert!(
        store
            .publish_generation_if_current(&expected, &winner_descriptor, &loser_index, &spatial)
            .expect("compare loser descriptor")
            .is_none()
    );
    let mut retry_entries = winner_index.entries.clone();
    retry_entries.extend(loser_index.entries);
    let retry_index = SceneCacheIndex {
        entries: retry_entries,
        ..winner_index
    };
    assert!(
        store
            .publish_generation_if_current(
                &winner_descriptor,
                &winner_descriptor,
                &retry_index,
                &spatial,
            )
            .expect("retry loser against current winner")
            .is_some()
    );
    let current = store
        .load_activation(scene_id)
        .expect("load winning activation")
        .expect("winning activation remains published");
    let paths = current
        .index
        .entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            SceneCacheEntryKind::OwnedPrim { prim_path } => Some(prim_path.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(paths, ["/SceneRoot/Winner", "/SceneRoot/Loser"]);
}

fn owned_entry(scene_id: SceneId, path: &str) -> SceneCacheEntry {
    SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::PrimPath(path.to_owned()),
        },
        parent: None,
        transform: CachedTransform::Placement(usd_project::ScenePlacementTransform::IDENTITY),
        bounds: None,
        cacheable: false,
        bim_enabled: false,
        geometry: None,
        material: None,
        animation: None,
        semantic_key: None,
        kind: SceneCacheEntryKind::OwnedPrim {
            prim_path: path.to_owned(),
        },
        content_hash: None,
    }
}
