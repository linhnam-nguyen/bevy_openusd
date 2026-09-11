use super::{OwnedPrimPathResolution, normalized_requested_paths, owned_prim_path_for_payload};
use crate::project::blob_store::{BlobStore, prepare_mesh_payload};
use crate::project::cache::SceneCacheStore;
use crate::project::cache_contract::{
    CachedTransform, SCENE_CACHE_INDEX_SCHEMA_VERSION, SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
    SceneCacheAddress, SceneCacheDescriptorV3, SceneCacheEntry, SceneCacheEntryKind,
    SceneCacheIndex, SceneCacheOccurrence, SceneCacheState, SceneSpatialIndex,
};
use tempfile::tempdir;
use usd_model::HashDigest;
use usd_project::{SceneId, SceneMemberId};

#[test]
fn requested_paths_are_canonical_and_deduplicated_before_cache_lookup() {
    let paths = normalized_requested_paths(&[
        " /SceneRoot/Member ".to_owned(),
        "/SceneRoot/Member/".to_owned(),
        "/".to_owned(),
    ])
    .expect("requested paths validate");
    assert_eq!(paths, vec!["/SceneRoot/Member"]);
}

#[test]
fn payload_lookup_uses_the_owner_scene_and_ignores_composed_only_rows() {
    let directory = tempdir().expect("Scene cache test directory");
    let scene_id = SceneId::new_v4();
    let blob_hash = HashDigest::new([8; HashDigest::BYTE_LEN]);
    let index = SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 4,
        entries: vec![
            SceneCacheEntry {
                address: SceneCacheAddress {
                    scene_id,
                    occurrence: SceneCacheOccurrence::Member(SceneMemberId::new_v4()),
                },
                parent: None,
                transform: CachedTransform::Placement(
                    usd_project::ScenePlacementTransform::IDENTITY,
                ),
                bounds: None,
                cacheable: false,
                bim_enabled: false,
                geometry: None,
                material: None,
                animation: None,
                semantic_key: None,
                kind: SceneCacheEntryKind::ChildScene {
                    scene_id: SceneId::new_v4(),
                    member_id: SceneMemberId::new_v4(),
                },
                content_hash: Some(blob_hash),
            },
            SceneCacheEntry {
                address: SceneCacheAddress {
                    scene_id,
                    occurrence: SceneCacheOccurrence::PrimPath("/SceneRoot/Owned".to_owned()),
                },
                parent: None,
                transform: CachedTransform::Placement(
                    usd_project::ScenePlacementTransform::IDENTITY,
                ),
                bounds: None,
                cacheable: false,
                bim_enabled: false,
                geometry: None,
                material: None,
                animation: None,
                semantic_key: None,
                kind: SceneCacheEntryKind::OwnedPrim {
                    prim_path: "/SceneRoot/Owned".to_owned(),
                },
                content_hash: Some(blob_hash),
            },
        ],
    };
    let spatial = SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 4,
        entries: Vec::new(),
    };
    let store = SceneCacheStore::new(directory.path());
    store
        .publish_generation(
            &SceneCacheDescriptorV3::invalidated(
                scene_id,
                4,
                HashDigest::new([9; HashDigest::BYTE_LEN]),
            ),
            &index,
            &spatial,
        )
        .expect("publish owner Scene cache");

    assert_eq!(
        owned_prim_path_for_payload(directory.path(), scene_id, blob_hash)
            .expect("resolve owner Scene payload"),
        OwnedPrimPathResolution::Ready("/SceneRoot/Owned".to_owned())
    );
    assert_eq!(
        owned_prim_path_for_payload(directory.path(), SceneId::new_v4(), blob_hash,)
            .expect("reject unknown owner Scene"),
        OwnedPrimPathResolution::Rejected
    );
}

#[test]
fn current_cache_ready_miss_escalates_to_targeted_scene_repair() {
    let directory = tempdir().expect("Scene cache test directory");
    let scene_id = SceneId::new_v4();
    let path = "/SceneRoot/Missing";
    let mut mesh = bevy::mesh::Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        bevy::mesh::Mesh::ATTRIBUTE_POSITION,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    mesh.insert_indices(bevy::mesh::Indices::U32(vec![0, 1, 2]));
    let prepared = prepare_mesh_payload(&mesh).expect("test mesh prepares");
    let blob_hash = HashDigest::from_hex(&prepared.blob_id.0).expect("mesh hash decodes");
    let index = SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 1,
        entries: vec![SceneCacheEntry {
            address: SceneCacheAddress {
                scene_id,
                occurrence: SceneCacheOccurrence::PrimPath(path.to_owned()),
            },
            parent: None,
            transform: CachedTransform::Placement(
                usd_project::ScenePlacementTransform::IDENTITY,
            ),
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
            content_hash: Some(blob_hash),
        }],
    };
    let spatial = SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 1,
        entries: Vec::new(),
    };
    let mut descriptor = SceneCacheDescriptorV3::invalidated(
        scene_id,
        1,
        HashDigest::new([10; HashDigest::BYTE_LEN]),
    );
    descriptor.state = SceneCacheState::Partial;
    let store = SceneCacheStore::new(directory.path());
    let expected = store
        .publish_generation(&descriptor, &index, &spatial)
        .expect("publish current owner cache");

    assert_eq!(
        super::persist_lookup_repair(directory.path(), scene_id, &expected, path, blob_hash)
            .expect("validate current CacheReady miss"),
        super::TargetedPersistenceOutcome::ScenePayloadMissing
    );
    assert!(matches!(
        super::persist_scene_payloads_for_repair(
            directory.path(),
            scene_id,
            &expected,
            vec![usd_bevy::TargetedRenderPayload {
                path: path.to_owned(),
                mesh,
                local_bounds: None,
            }],
        )
        .expect("persist exact targeted Scene repair"),
        super::TargetedPersistenceOutcome::Published {
            lookup_repaired: false,
            ..
        }
    ));
    let activation = store
        .load_activation(scene_id)
        .expect("read repaired activation")
        .expect("repaired Scene cache is published");
    let entry = activation
        .index
        .entries
        .iter()
        .find(|entry| matches!(&entry.kind, SceneCacheEntryKind::OwnedPrim { prim_path } if prim_path == path))
        .expect("owner entry remains addressable");
    let geometry = entry.geometry.as_ref().expect("payload was repaired");
    assert!(
        store
            .object_store(scene_id)
            .expect("open owning Scene object store")
            .get(&geometry.blob_id)
            .expect("read repaired geometry")
            .is_some()
    );
}
