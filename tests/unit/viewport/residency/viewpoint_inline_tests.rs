use super::*;
use crate::project::cache_contract::{
    CachedTransform, SceneCacheAddress, SceneCacheBlobRef, SceneCacheEntry, SceneCacheEntryKind,
    SceneCacheOccurrence, SceneCacheState,
};
use usd_model::{BlobId, Bounds3, HashDigest};
use usd_project::ScenePlacementTransform;

fn key(scene_id: SceneId, value: u8) -> ScenePayloadKey {
    ScenePayloadKey {
        scene_id,
        blob_hash: HashDigest::new([value; HashDigest::BYTE_LEN]),
    }
}

fn payload(scene_id: SceneId, value: u8, generation: u64) -> ViewpointPayload {
    ViewpointPayload {
        key: key(scene_id, value),
        generation,
        cpu_bytes: 2,
        gpu_bytes: 2,
    }
}

#[test]
fn mounted_viewpoint_resolves_only_owned_cache_geometry() {
    let scene = SceneId::new_v4();
    let hash = HashDigest::new([7; HashDigest::BYTE_LEN]);
    let presentation = SceneCachePresentation {
        scene_id: scene,
        generation: 4,
        state: SceneCacheState::Ready,
        entries: vec![SceneCacheEntry {
            address: SceneCacheAddress {
                scene_id: scene,
                occurrence: SceneCacheOccurrence::PrimPath("/World/Camera".to_owned()),
            },
            parent: None,
            transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
            bounds: Some(Bounds3 {
                min: [-1.0; 3],
                max: [1.0; 3],
            }),
            cacheable: true,
            bim_enabled: false,
            geometry: Some(SceneCacheBlobRef {
                blob_id: BlobId(hash.to_hex()),
                byte_size: 8,
            }),
            material: None,
            animation: None,
            semantic_key: None,
            kind: SceneCacheEntryKind::OwnedPrim {
                prim_path: "/World/Camera".to_owned(),
            },
            content_hash: Some(hash),
        }],
    };

    let resolved = viewpoint_payload(&presentation, "/World/Camera").expect("owned cache");
    assert_eq!(resolved.key.scene_id, scene);
    assert_eq!(resolved.key.blob_hash, hash);
    assert!(viewpoint_payload(&presentation, "/World/Other").is_none());
}

#[test]
fn viewpoint_reason_coexists_and_releases_on_switch() {
    let scene = SceneId::new_v4();
    let first = payload(scene, 1, 3);
    let second = payload(scene, 2, 3);
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, 3, Vec::new());
    assert!(authority.request_reason(
        first.key,
        ResidencyReason::CameraNear,
        3,
        first.cpu_bytes,
        first.gpu_bytes,
    ));
    let mut state = ActiveViewpointResidencyState::default();
    state.set_active(Some(first), &mut authority);
    assert!(
        authority
            .reasons(&first.key)
            .is_some_and(|reasons| reasons.contains(&ResidencyReason::CameraNear))
    );
    state.set_active(Some(second), &mut authority);
    assert!(
        authority
            .reasons(&first.key)
            .is_some_and(|reasons| !reasons.contains(&ResidencyReason::ActiveViewpoint))
    );
    assert!(
        authority
            .reasons(&second.key)
            .is_some_and(|reasons| reasons.contains(&ResidencyReason::ActiveViewpoint))
    );
}

#[test]
fn stale_viewpoint_generation_is_rejected() {
    let scene = SceneId::new_v4();
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, 4, Vec::new());
    let mut state = ActiveViewpointResidencyState::default();
    state.set_active(Some(payload(scene, 1, 3)), &mut authority);
    assert!(state.active.is_none());
    assert!(authority.reasons(&key(scene, 1)).is_none());
}
