use super::*;
use crate::project::cache_contract::{
    CachedTransform, SCENE_CACHE_INDEX_SCHEMA_VERSION, SceneCacheAddress, SceneCacheBlobRef,
    SceneCacheEntry, SceneCacheOccurrence,
};
use usd_project::ScenePlacementTransform;

#[test]
fn child_flags_are_computed_in_one_parent_index_pass() {
    let mut app = App::new();
    app.init_resource::<CurrentHierarchyProjection>();
    let scene_id = usd_project::SceneId::new_v4();
    let root = SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::PrimPath("/SceneRoot/Root".into()),
        },
        parent: None,
        transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
        bounds: None,
        cacheable: false,
        bim_enabled: false,
        geometry: None,
        material: None,
        animation: None,
        semantic_key: None,
        kind: SceneCacheEntryKind::OwnedPrim {
            prim_path: "/SceneRoot/Root".into(),
        },
        content_hash: None,
    };
    let child = SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::PrimPath("/SceneRoot/Root/Child".into()),
        },
        parent: Some(0),
        transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
        bounds: None,
        cacheable: true,
        bim_enabled: false,
        geometry: Some(SceneCacheBlobRef {
            blob_id: usd_model::BlobId("a".repeat(64)),
            byte_size: 1,
        }),
        material: None,
        animation: None,
        semantic_key: None,
        kind: SceneCacheEntryKind::OwnedPrim {
            prim_path: "/SceneRoot/Root/Child".into(),
        },
        content_hash: Some(usd_model::HashDigest::from_hex(&"a".repeat(64)).unwrap()),
    };
    let mut descriptor = crate::project::cache::SceneCacheDescriptorV3::invalidated(
        scene_id,
        1,
        usd_model::HashDigest::from_hex(&"b".repeat(64)).unwrap(),
    );
    descriptor.state = crate::project::cache_contract::SceneCacheState::Ready;
    descriptor.prim_count = 2;
    descriptor.cacheable_count = 1;
    let activation = SceneCacheActivation {
        descriptor,
        index: crate::project::cache_contract::SceneCacheIndex {
            schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
            scene_id,
            generation: 1,
            entries: vec![root, child],
        },
    };
    publish_scene_cache_presentation(&mut app.world_mut(), &activation, None);
    let projection = app.world().resource::<CurrentHierarchyProjection>();
    assert!(
        projection
            .snapshot()
            .nodes
            .iter()
            .any(|node| node.has_children)
    );
}
