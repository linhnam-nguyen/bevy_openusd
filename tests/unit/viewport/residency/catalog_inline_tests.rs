use super::*;
use crate::project::cache_contract::{CachedTransform, SceneCacheEntry, SceneCacheOccurrence};
use usd_model::{BlobId, HashDigest};

fn entry(scene_id: SceneId, material: Option<SceneCacheBlobRef>) -> SceneCacheEntry {
    let hash = HashDigest::new([7; HashDigest::BYTE_LEN]);
    SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::PrimPath("/World/Mesh".to_owned()),
        },
        parent: None,
        transform: CachedTransform::Placement(usd_project::ScenePlacementTransform::IDENTITY),
        bounds: None,
        cacheable: true,
        bim_enabled: false,
        geometry: Some(SceneCacheBlobRef {
            blob_id: BlobId(hash.to_string()),
            byte_size: 1,
        }),
        material,
        animation: None,
        semantic_key: None,
        kind: SceneCacheEntryKind::OwnedPrim {
            prim_path: "/World/Mesh".to_owned(),
        },
        content_hash: Some(hash),
    }
}

#[test]
fn reason_masks_are_minimal_and_animation_is_additive() {
    assert_eq!(
        PayloadLoadMask::for_reason(ResidencyReason::CameraNear),
        PayloadLoadMask {
            geometry: true,
            material: true,
            animation: false,
        }
    );
    assert_eq!(
        PayloadLoadMask::for_reason(ResidencyReason::AnimationRequired),
        PayloadLoadMask {
            geometry: true,
            material: true,
            animation: true,
        }
    );
}

#[test]
fn catalog_rejects_invalid_scene_blob_references() {
    let scene_id = SceneId::new_v4();
    let presentation = SceneCachePresentation {
        scene_id,
        generation: 4,
        state: crate::project::cache_contract::SceneCacheState::Ready,
        entries: vec![entry(
            scene_id,
            Some(SceneCacheBlobRef {
                blob_id: BlobId(String::new()),
                byte_size: 1,
            }),
        )],
    };

    assert!(ScenePayloadCatalog::from_presentation(&presentation).is_err());
}
