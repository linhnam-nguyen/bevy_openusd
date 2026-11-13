use super::*;
use bevy::camera::primitives::Frustum;
use bevy::math::Vec4;
use bevy::math::primitives::{HalfSpace, ViewFrustum};
use usd_model::{BlobId, HashDigest};
use usd_project::{SceneId, SceneMemberId, ScenePlacementTransform};

use crate::viewport::residency::camera::CameraSample;

fn payload(x: f64, key: u8) -> SceneSpatialPayload {
    let scene_id = SceneId::new_v4();
    SceneSpatialPayload {
        address: SceneCacheAddress {
            scene_id,
            occurrence: crate::project::cache_contract::SceneCacheOccurrence::Member(
                SceneMemberId::new_v4(),
            ),
        },
        payload_key: ScenePayloadKey {
            scene_id,
            blob_hash: HashDigest::new([key; HashDigest::BYTE_LEN]),
        },
        transform: ScenePlacementTransform::IDENTITY,
        bounds: Bounds3 {
            min: [x, 0.0, 0.0],
            max: [x + 1.0, 1.0, 1.0],
        },
        cpu_bytes: 4,
        gpu_bytes: 8,
    }
}

#[test]
fn rtree_returns_only_intersecting_camera_candidates() {
    let near = payload(0.0, 1);
    let far = payload(100.0, 2);
    let index = CameraCandidateIndex::from_payloads(vec![near.clone(), far]);
    let found = index
        .candidates(CameraRegion::around([0.0, 0.0, 0.0], 2.0, 0.0))
        .map(|entry| entry.payload_key.clone())
        .collect::<Vec<_>>();
    assert_eq!(found, vec![near.payload_key]);
}

#[test]
fn non_finite_bounds_are_normalized_to_a_safe_origin_box() {
    let mut entry = payload(0.0, 3);
    entry.bounds = Bounds3 {
        min: [f64::NAN; 3],
        max: [f64::INFINITY; 3],
    };
    let index = CameraCandidateIndex::from_payloads(vec![entry]);
    assert_eq!(index.len(), 1);
    assert_eq!(
        index
            .candidates(CameraRegion::around([0.0; 3], 1.0, 0.0))
            .count(),
        1
    );
}

#[test]
fn scene_payloads_compose_ancestor_transform_for_camera_space() {
    let scene_id = SceneId::new_v4();
    let parent_transform = ScenePlacementTransform::from_trs(
        [10.0, 2.0, 0.0],
        [
            std::f64::consts::FRAC_1_SQRT_2,
            0.0,
            0.0,
            std::f64::consts::FRAC_1_SQRT_2,
        ],
        [2.0, 1.0, 1.0],
    );
    let child_transform = ScenePlacementTransform::from_translation([1.0, 0.0, 0.0]);
    let parent_address = SceneCacheAddress {
        scene_id,
        occurrence: crate::project::cache_contract::SceneCacheOccurrence::PrimPath(
            "/SceneRoot/Parent".to_owned(),
        ),
    };
    let child_address = SceneCacheAddress {
        scene_id,
        occurrence: crate::project::cache_contract::SceneCacheOccurrence::PrimPath(
            "/SceneRoot/Parent/Child".to_owned(),
        ),
    };
    let parent = cache_entry(
        parent_address,
        None,
        CachedTransform::Placement(parent_transform),
        None,
    );
    let child = cache_entry(
        child_address,
        Some(0),
        CachedTransform::Placement(child_transform),
        Some(Bounds3 {
            min: [0.0; 3],
            max: [1.0; 3],
        }),
    );
    let payloads = scene_payloads(&[parent, child]);
    assert_eq!(payloads.len(), 1);
    let payload = &payloads[0];
    let expected = Mat4::from_cols_array(&parent_transform.0.map(|value| value as f32))
        * Mat4::from_cols_array(&child_transform.0.map(|value| value as f32));
    assert_eq!(payload.transform.0, expected.to_cols_array().map(f64::from));
    assert!((payload.bounds.min[0] - 9.0).abs() <= 1e-5);
    assert!(payload.bounds.min[1] > 1.0);
    assert!(payload.admitted_to(&admission([10.0, 4.0, 0.0])));
    assert!(!payload.admitted_to(&admission([0.0; 3])));
    let index = CameraCandidateIndex::from_payloads(payloads);
    assert_eq!(
        index
            .candidates(CameraRegion::around([10.0, 4.0, 0.0], 1.0, 0.0))
            .count(),
        1
    );
    assert_eq!(
        index
            .candidates(CameraRegion::around([0.0; 3], 1.0, 0.0))
            .count(),
        0
    );
}

fn cache_entry(
    address: SceneCacheAddress,
    parent: Option<u32>,
    transform: CachedTransform,
    bounds: Option<Bounds3>,
) -> SceneCacheEntry {
    let content_hash = HashDigest::new([8; HashDigest::BYTE_LEN]);
    SceneCacheEntry {
        address,
        parent,
        transform,
        bounds,
        cacheable: true,
        bim_enabled: false,
        geometry: Some(crate::project::cache_contract::SceneCacheBlobRef {
            blob_id: BlobId("8".repeat(64)),
            byte_size: 4,
        }),
        material: None,
        animation: None,
        semantic_key: None,
        kind: crate::project::cache_contract::SceneCacheEntryKind::OwnedPrim {
            prim_path: "/SceneRoot/Test".to_owned(),
        },
        content_hash: Some(content_hash),
    }
}

fn admission(position: [f64; 3]) -> CameraAdmission {
    CameraAdmission {
        sample: CameraSample {
            position,
            search_radius: 0.0,
            preload_margin: 0.0,
            ..CameraSample::default()
        },
        frustum: Frustum(ViewFrustum {
            half_spaces: [HalfSpace::new(Vec4::new(1.0, 0.0, 0.0, f32::INFINITY)); 6],
        }),
        section_box: None,
    }
}
