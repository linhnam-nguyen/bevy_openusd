use super::*;
use crate::project::cache_contract::{CachedTransform, SceneCacheBlobRef, SceneCacheOccurrence};
use usd_project::ScenePlacementTransform;

fn owned_entry(scene_id: SceneId, path: &str, byte: u8) -> SceneCacheEntry {
    let hex = format!("{byte:02x}").repeat(HashDigest::BYTE_LEN);
    let content_hash = HashDigest::from_hex(&hex).expect("test digest is valid");
    SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::PrimPath(path.to_owned()),
        },
        parent: None,
        transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
        bounds: None,
        cacheable: true,
        bim_enabled: false,
        geometry: Some(SceneCacheBlobRef {
            blob_id: usd_model::BlobId(hex),
            byte_size: 1,
        }),
        material: None,
        animation: None,
        semantic_key: None,
        kind: SceneCacheEntryKind::OwnedPrim {
            prim_path: path.to_owned(),
        },
        content_hash: Some(content_hash),
    }
}

fn child_entry(parent_scene: SceneId, member_id: SceneMemberId, child: SceneId) -> SceneCacheEntry {
    SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id: parent_scene,
            occurrence: SceneCacheOccurrence::Member(member_id),
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
        kind: SceneCacheEntryKind::ChildScene {
            scene_id: child,
            member_id,
        },
        content_hash: None,
    }
}

fn index(scene_id: SceneId, generation: u64, entries: Vec<SceneCacheEntry>) -> SceneCacheIndex {
    let mut entries = entries;
    entries.sort_by(|left, right| left.address.cmp(&right.address));
    SceneCacheIndex {
        schema_version: super::super::SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation,
        entries,
    }
}

#[test]
fn replacing_one_scene_keeps_direct_nested_routes_and_unrelated_rows() -> Result<()> {
    let parent = SceneId::new_v4();
    let child = SceneId::new_v4();
    let unrelated = SceneId::new_v4();
    let member = SceneMemberId::new_v4();
    let parent_index = index(parent, 1, vec![child_entry(parent, member, child)]);
    let child_index = index(child, 4, vec![owned_entry(child, "/SceneRoot/Leaf", 1)]);
    let unrelated_index = index(
        unrelated,
        8,
        vec![owned_entry(unrelated, "/SceneRoot/Other", 2)],
    );
    let mut lookup = ProjectCacheLookup::from_scene_indexes(&[
        parent_index.clone(),
        child_index.clone(),
        unrelated_index.clone(),
    ])?;

    assert_eq!(lookup.generation(child), Some(4));
    assert_eq!(lookup.child_scene_for_member(parent, member), Some(child));
    assert!(lookup.get(&child_index.entries[0].address).is_some());
    assert!(lookup.get(&unrelated_index.entries[0].address).is_some());

    let replacement = index(parent, 2, vec![child_entry(parent, member, child)]);
    lookup.replace_scene_index(&replacement)?;

    assert_eq!(lookup.generation(parent), Some(2));
    assert_eq!(lookup.child_scene_for_member(parent, member), Some(child));
    assert!(lookup.get(&child_index.entries[0].address).is_some());
    assert!(lookup.get(&unrelated_index.entries[0].address).is_some());
    Ok(())
}
