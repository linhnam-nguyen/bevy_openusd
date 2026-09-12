use super::*;
use crate::project::cache_contract::{
    CachedTransform, SceneCacheAddress, SceneCacheBlobRef, SceneCacheEntry, SceneCacheEntryKind,
    SceneCacheOccurrence, SceneCacheState,
};
use crate::project::cache_scene_payload::{SceneAnimationBlob, SCENE_ANIMATION_VERSION};
use usd_model::{BlobId, Bounds3, HashDigest};
use usd_project::{SceneMemberId, ScenePlacementTransform};

fn key(scene_id: SceneId, value: u8) -> ScenePayloadKey {
    ScenePayloadKey {
        scene_id,
        blob_hash: HashDigest::new([value; HashDigest::BYTE_LEN]),
    }
}

fn candidates(scene_id: SceneId, count: u8, generation: u64) -> Vec<AnimationPayload> {
    (0..count)
        .map(|value| AnimationPayload {
            key: key(scene_id, value),
            generation,
            cpu_bytes: 1,
            gpu_bytes: 1,
        })
        .collect()
}

fn payloads(
    scene_id: SceneId,
    generation: u64,
    address: Option<SceneCacheAddress>,
) -> SceneAnimationPayloads {
    let mut payloads = SceneAnimationPayloads {
        scene_id: Some(scene_id),
        generation: Some(generation),
        ..Default::default()
    };
    if let Some(address) = address {
        payloads.by_address.insert(
            address,
            SceneAnimationBlob {
                version: SCENE_ANIMATION_VERSION,
                source_path: String::new(),
                joint_order: Vec::new(),
                blend_shape_order: Vec::new(),
                samples: Vec::new(),
            },
        );
    }
    payloads
}

fn animated_entry(
    scene_id: SceneId,
    value: u8,
    cacheable: bool,
    kind: SceneCacheEntryKind,
) -> SceneCacheEntry {
    let hash = key(scene_id, value).blob_hash;
    SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::PrimPath(format!("/World/Node{value}")),
        },
        parent: None,
        transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
        bounds: Some(Bounds3 {
            min: [-1.0; 3],
            max: [1.0; 3],
        }),
        cacheable,
        bim_enabled: false,
        geometry: Some(SceneCacheBlobRef {
            blob_id: BlobId(hash.to_hex()),
            byte_size: 8,
        }),
        material: None,
        animation: Some(SceneCacheBlobRef {
            blob_id: BlobId(hash.to_hex()),
            byte_size: 8,
        }),
        semantic_key: None,
        kind,
        content_hash: Some(hash),
    }
}

#[test]
fn rebuild_requires_cacheable_owned_prim_animation_rows() {
    let scene = SceneId::new_v4();
    let valid = animated_entry(
        scene,
        1,
        true,
        SceneCacheEntryKind::OwnedPrim {
            prim_path: "/World/Node1".to_owned(),
        },
    );
    let non_cacheable = animated_entry(
        scene,
        2,
        false,
        SceneCacheEntryKind::OwnedPrim {
            prim_path: "/World/Node2".to_owned(),
        },
    );
    let child_scene = animated_entry(
        scene,
        3,
        true,
        SceneCacheEntryKind::ChildScene {
            scene_id: SceneId::new_v4(),
            member_id: SceneMemberId::new_v4(),
        },
    );
    let presentation = SceneCachePresentation {
        scene_id: scene,
        generation: 7,
        state: SceneCacheState::Ready,
        entries: vec![valid, non_cacheable, child_scene],
    };
    let mut state = AnimationResidencyState::default();

    let cached_payloads = payloads(scene, 7, Some(presentation.entries[0].address.clone()));
    state.rebuild(&presentation, Some(&cached_payloads));

    assert_eq!(state.candidates.len(), 1);
    assert_eq!(state.candidates[0].key, key(scene, 1));
}

#[test]
fn rebuild_requires_matching_scene_animation_payload_identity() {
    let scene = SceneId::new_v4();
    let entry = animated_entry(
        scene,
        1,
        true,
        SceneCacheEntryKind::OwnedPrim {
            prim_path: "/World/Node1".to_owned(),
        },
    );
    let presentation = SceneCachePresentation {
        scene_id: scene,
        generation: 7,
        state: SceneCacheState::Ready,
        entries: vec![entry.clone()],
    };
    let cases = [
        ("none", None, 0),
        (
            "wrong scene",
            Some(payloads(SceneId::new_v4(), 7, Some(entry.address.clone()))),
            0,
        ),
        (
            "wrong generation",
            Some(payloads(scene, 8, Some(entry.address.clone()))),
            0,
        ),
        ("missing address", Some(payloads(scene, 7, None)), 0),
        (
            "matching address",
            Some(payloads(scene, 7, Some(entry.address.clone()))),
            1,
        ),
    ];
    for (case, cached_payloads, expected) in cases {
        let mut state = AnimationResidencyState::default();
        state.rebuild(&presentation, cached_payloads.as_ref());
        assert_eq!(state.candidates.len(), expected, "{case}");
    }
}

#[test]
fn hydration_error_or_source_fallback_is_fail_closed() {
    let scene = SceneId::new_v4();
    let presentation = SceneCachePresentation {
        scene_id: scene,
        generation: 7,
        state: SceneCacheState::FallbackRequired,
        entries: vec![animated_entry(
            scene,
            1,
            true,
            SceneCacheEntryKind::OwnedPrim {
                prim_path: "/World/Node1".to_owned(),
            },
        )],
    };
    let mut state = AnimationResidencyState::default();
    // Hydration errors and source fallback do not install payload evidence.
    state.rebuild(&presentation, None);
    assert!(state.candidates.is_empty());
}

#[test]
fn animation_window_is_bounded_and_rotates_fairly() {
    let scene = SceneId::new_v4();
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, 7, Vec::new());
    let mut state = AnimationResidencyState {
        scene_id: Some(scene),
        generation: Some(7),
        candidates: candidates(scene, 96, 7),
        ..Default::default()
    };

    state.advance(&mut authority);
    assert_eq!(state.active.len(), ANIMATION_LOOKAHEAD);
    assert_eq!(authority.queue_len(), ANIMATION_LOOKAHEAD);
    state.advance(&mut authority);
    assert_eq!(state.active.len(), ANIMATION_LOOKAHEAD);
    assert!(state.active.contains_key(&key(scene, 64)));
    assert!(!state.active.contains_key(&key(scene, 95)));
}

#[test]
fn stale_animation_generation_is_rejected_without_a_reason() {
    let scene = SceneId::new_v4();
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, 8, Vec::new());
    let mut state = AnimationResidencyState {
        scene_id: Some(scene),
        generation: Some(7),
        candidates: candidates(scene, 1, 7),
        ..Default::default()
    };

    state.advance(&mut authority);
    assert!(state.active.is_empty());
    assert!(authority.reasons(&key(scene, 0)).is_none());
}
