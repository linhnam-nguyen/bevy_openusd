use super::*;
use crate::project::cache_contract::{SceneCacheAddress, SceneCacheOccurrence};
use crate::viewport::residency::{camera::CameraAdmission, spatial::SceneSpatialPayload};
use bevy::asset::Assets;
use bevy::camera::primitives::Frustum;
use bevy::math::primitives::{HalfSpace, ViewFrustum};
use bevy::math::Vec4;
use bevy::mesh::{Indices, Mesh};
use usd_model::Bounds3;
use usd_project::{SceneMemberId, ScenePlacementTransform};

fn key(scene_id: SceneId, value: u8) -> ScenePayloadKey {
    ScenePayloadKey {
        scene_id,
        blob_hash: HashDigest::new([value; HashDigest::BYTE_LEN]),
    }
}

fn candidate(scene_id: SceneId, value: u8, x: f64) -> SceneSpatialPayload {
    SceneSpatialPayload {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::Member(SceneMemberId::new_v4()),
        },
        payload_key: key(scene_id, value),
        transform: ScenePlacementTransform::IDENTITY,
        bounds: Bounds3 {
            min: [x, -1.0, -1.0],
            max: [x + 1.0, 1.0, 1.0],
        },
        cpu_bytes: 8,
        gpu_bytes: 8,
    }
}

fn camera_admission(sample: CameraSample) -> CameraAdmission {
    CameraAdmission {
        sample,
        frustum: Frustum(ViewFrustum {
            half_spaces: [HalfSpace::new(Vec4::new(1.0, 0.0, 0.0, f32::INFINITY)); 6],
        }),
        section_box: None,
    }
}

#[test]
fn camera_and_selection_share_one_load_but_keep_two_reasons() {
    let scene = SceneId::new_v4();
    let payload = candidate(scene, 1, 0.0);
    let key = payload.payload_key;
    let mut authority = ResidencyAuthority::with_budgets(ResidencyBudgets {
        cpu_bytes: 64,
        gpu_bytes: 64,
        upload_bytes_per_frame: 64,
    });
    let mut assets = Assets::<Mesh>::default();
    authority.install_scene(scene, 7, vec![payload]);
    assert!(authority.update_camera_near(camera_admission(CameraSample::default())));
    assert_eq!(authority.queue_len(), 1);
    assert!(authority.request_reason(key, ResidencyReason::Selected, 7, 8, 8));
    assert_eq!(authority.queue_len(), 1);
    assert_eq!(authority.reasons(&key).unwrap().len(), 2);
    let job = authority.begin_next_load().unwrap();
    assert!(authority.complete_cpu(job.key, 7, 8, 8));
    assert_eq!(authority.pump_uploads(&mut assets, None), vec![key]);
    assert!(authority.remove_reason(key, ResidencyReason::Selected, 7));
    assert_eq!(
        authority.state(&key),
        Some(PayloadResidencyState::GpuResident)
    );
    assert!(
        authority
            .reasons(&key)
            .unwrap()
            .contains(&ResidencyReason::CameraNear)
    );
}

#[test]
fn installing_a_new_generation_retires_old_state_and_readmits_same_hash() {
    let scene = SceneId::new_v4();
    let payload = candidate(scene, 9, 0.0);
    let key = payload.payload_key;
    let mut authority = ResidencyAuthority::default();

    authority.install_scene(scene, 11, vec![payload.clone()]);
    assert!(authority.request_reason(key, ResidencyReason::Selected, 11, 8, 8));
    let stale_job = authority.begin_next_load().unwrap();

    authority.install_scene(scene, 12, vec![payload]);
    assert_eq!(authority.state(&key), None);
    assert_eq!(authority.queue_len(), 0);
    assert_eq!(authority.accounted_bytes(), (0, 0));
    assert!(!authority.complete_cpu(stale_job.key, stale_job.generation, 8, 8));

    assert!(authority.request_reason(key, ResidencyReason::Selected, 12, 8, 8));
    let current_job = authority.begin_next_load().unwrap();
    assert_eq!(current_job.generation, 12);
    assert!(authority.complete_cpu(current_job.key, 12, 8, 8));
}

#[test]
fn generation_mismatch_rejects_stale_completion_and_budget_pressure_evicts_warm() {
    let scene = SceneId::new_v4();
    let first = candidate(scene, 1, 0.0);
    let second = candidate(scene, 2, 10.0);
    let mut authority = ResidencyAuthority::with_budgets(ResidencyBudgets {
        cpu_bytes: 8,
        gpu_bytes: 8,
        upload_bytes_per_frame: 8,
    });
    authority.install_scene(scene, 4, vec![first.clone(), second.clone()]);
    assert!(authority.request_reason(first.payload_key, ResidencyReason::Selected, 4, 8, 8));
    let job = authority.begin_next_load().unwrap();
    assert!(!authority.complete_cpu(job.key, 3, 8, 8));
    assert!(authority.complete_cpu(job.key, 4, 8, 8));
    let mut assets = Assets::<Mesh>::default();
    assert_eq!(
        authority.pump_uploads(&mut assets, None),
        vec![first.payload_key]
    );
    assert!(authority.remove_reason(first.payload_key, ResidencyReason::Selected, 4));
    assert_eq!(authority.warm_len(), 1);
    assert!(authority.request_reason(second.payload_key, ResidencyReason::Selected, 4, 8, 8));
    let second_job = authority.begin_next_load().unwrap();
    assert!(authority.complete_cpu(second_job.key, 4, 8, 8));
    assert_eq!(
        authority.pump_uploads(&mut assets, None),
        vec![second.payload_key]
    );
    assert_eq!(
        authority.state(&first.payload_key),
        Some(PayloadResidencyState::Unloaded)
    );
}

#[test]
fn cache_presentation_retirement_blocks_old_camera_and_completion_publication() {
    let scene = SceneId::new_v4();
    let payload = candidate(scene, 3, 0.0);
    let key = payload.payload_key;
    let mut authority = ResidencyAuthority::default();

    authority.install_scene(scene, 21, vec![payload.clone()]);
    assert!(authority.update_camera_near(camera_admission(CameraSample::default())));
    let stale_job = authority
        .begin_next_load()
        .expect("camera admission queues work");

    authority.retire();
    assert_eq!(authority.state(&key), None);
    assert_eq!(authority.queue_len(), 0);
    assert_eq!(authority.accounted_bytes(), (0, 0));
    assert!(!authority.complete_cached_cpu(
        stale_job,
        Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        )
    ));
    assert!(!authority.update_camera_near(camera_admission(CameraSample::default())));

    authority.install_scene(scene, 22, vec![payload]);
    assert!(authority.update_camera_near(camera_admission(CameraSample::default())));
    assert_eq!(authority.queue_len(), 1);
}

#[test]
fn camera_near_cached_payload_acquires_and_releases_renderer_ownership() {
    let scene = SceneId::new_v4();
    let payload = candidate(scene, 4, 0.0);
    let key = payload.payload_key;
    let mut authority = ResidencyAuthority::default();
    let mut assets = Assets::<Mesh>::default();

    authority.install_scene(scene, 31, vec![payload]);
    assert!(authority.request_reason(key, ResidencyReason::CameraNear, 31, 8, 8));
    let job = authority
        .begin_next_load()
        .expect("camera admission queues work");
    assert!(authority.complete_cpu(job.key, job.generation, 8, 8));
    assert_eq!(authority.pump_uploads(&mut assets, None), vec![key]);
    let asset_id = authority.render_asset_id(&key).expect("render ownership");
    assert!(assets.contains(asset_id));

    assert!(authority.remove_reason(key, ResidencyReason::CameraNear, 31));
    authority.set_budgets(ResidencyBudgets {
        cpu_bytes: 0,
        gpu_bytes: 0,
        upload_bytes_per_frame: 8,
    });
    let released = authority.take_released_render_assets();
    assert!(released.contains(&asset_id));
    for released_id in released {
        let _ = assets.remove(released_id);
    }
    assert!(!assets.contains(asset_id));
}

#[test]
fn active_camera_payloads_defer_until_cpu_and_gpu_capacity_is_released() {
    let scene = SceneId::new_v4();
    let first = candidate(scene, 5, 0.0);
    let second = candidate(scene, 6, 4.0);
    let mut authority = ResidencyAuthority::with_budgets(ResidencyBudgets {
        cpu_bytes: 8,
        gpu_bytes: 8,
        upload_bytes_per_frame: 16,
    });
    let mut assets = Assets::<Mesh>::default();
    authority.install_scene(scene, 41, vec![first.clone(), second.clone()]);
    assert!(authority.request_reason(first.payload_key, ResidencyReason::CameraNear, 41, 8, 8,));
    assert!(authority.request_reason(second.payload_key, ResidencyReason::CameraNear, 41, 8, 8,));

    let first_job = authority.begin_next_load().expect("first payload fits");
    assert!(authority.begin_next_load().is_none());
    assert!(authority.complete_cpu(first_job.key, first_job.generation, 8, 8));
    assert_eq!(
        authority.pump_uploads(&mut assets, None),
        vec![first.payload_key]
    );
    assert_eq!(authority.accounted_bytes(), (8, 8));
    assert_eq!(
        authority.state(&second.payload_key),
        Some(PayloadResidencyState::Queued)
    );

    assert!(authority.remove_reason(first.payload_key, ResidencyReason::CameraNear, 41));
    let second_job = authority
        .begin_next_load()
        .expect("released warm payload makes room");
    assert_eq!(
        authority.state(&first.payload_key),
        Some(PayloadResidencyState::Unloaded)
    );
    assert!(authority.complete_cpu(second_job.key, second_job.generation, 8, 8));
    assert_eq!(
        authority.pump_uploads(&mut assets, None),
        vec![second.payload_key]
    );
    let (cpu_used, gpu_used) = authority.accounted_bytes();
    assert!(cpu_used <= authority.budgets().cpu_bytes);
    assert!(gpu_used <= authority.budgets().gpu_bytes);
}

#[test]
fn decoded_mesh_footprint_replaces_encoded_estimate_before_accounting() {
    let scene = SceneId::new_v4();
    let encoded_bytes = 1;
    let mut payload = candidate(scene, 8, 0.0);
    payload.cpu_bytes = encoded_bytes;
    payload.gpu_bytes = encoded_bytes;
    let key = payload.payload_key;
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0, 0.0, 0.0]; 3]);
    mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
    let (decoded_cpu, decoded_gpu) = super::authority_loading::resident_mesh_footprint(&mesh);
    assert!(decoded_cpu > encoded_bytes);
    assert!(decoded_gpu > encoded_bytes);
    let mut authority = ResidencyAuthority::with_budgets(ResidencyBudgets {
        cpu_bytes: decoded_cpu,
        gpu_bytes: decoded_gpu,
        upload_bytes_per_frame: usize::MAX,
    });
    let mut assets = Assets::<Mesh>::default();
    authority.install_scene(scene, 61, vec![payload]);
    assert!(authority.request_reason(key, ResidencyReason::CameraNear, 61, encoded_bytes, encoded_bytes));
    let job = authority.begin_next_load().expect("encoded estimate fits");
    assert!(authority.complete_cached_cpu(job, mesh));
    assert_eq!(authority.pump_uploads(&mut assets, None), vec![key]);
    assert_eq!(authority.accounted_bytes(), (decoded_cpu, decoded_gpu));
    assert!(authority.accounted_bytes().0 <= authority.budgets().cpu_bytes);
    assert!(authority.accounted_bytes().1 <= authority.budgets().gpu_bytes);
}

#[test]
fn terminal_cache_failure_is_suppressed_until_new_scene_generation() {
    let scene = SceneId::new_v4();
    let payload = candidate(scene, 9, 0.0);
    let key = payload.payload_key;
    let mut authority = ResidencyAuthority::default();
    let sample = CameraSample::default();
    authority.install_scene(scene, 71, vec![payload.clone()]);
    assert!(authority.update_camera_near(camera_admission(sample)));
    let failed_job = authority.begin_next_load().expect("cache miss is dispatched");
    assert!(authority.suppress_failed_load(&failed_job));
    assert_eq!(authority.state(&key), Some(PayloadResidencyState::Unloaded));
    assert!(!authority.update_camera_near(camera_admission(sample)));
    assert_eq!(authority.queue_len(), 0);

    authority.install_scene(scene, 72, vec![payload]);
    assert!(authority.update_camera_near(camera_admission(sample)));
    assert_eq!(authority.begin_next_load().map(|job| job.generation), Some(72));
}

#[test]
fn oversized_single_payload_is_skipped_without_budget_overflow() {
    let scene = SceneId::new_v4();
    let payload = candidate(scene, 7, 0.0);
    let mut authority = ResidencyAuthority::with_budgets(ResidencyBudgets {
        cpu_bytes: 4,
        gpu_bytes: 4,
        upload_bytes_per_frame: 8,
    });
    authority.install_scene(scene, 42, vec![payload.clone()]);
    assert!(authority.request_reason(payload.payload_key, ResidencyReason::CameraNear, 42, 8, 8));
    assert!(authority.begin_next_load().is_none());
    assert_eq!(
        authority.state(&payload.payload_key),
        Some(PayloadResidencyState::Unloaded)
    );
    assert_eq!(authority.accounted_bytes(), (0, 0));
}

#[test]
fn stationary_camera_refills_active_demand_after_loader_saturation() {
    let scene = SceneId::new_v4();
    let mut payloads = Vec::with_capacity(DEFAULT_LOADER_CAPACITY + 1);
    for index in 0..=DEFAULT_LOADER_CAPACITY {
        let mut digest = [0; HashDigest::BYTE_LEN];
        digest[..8].copy_from_slice(&(index as u64).to_le_bytes());
        payloads.push(SceneSpatialPayload {
            address: SceneCacheAddress {
                scene_id: scene,
                occurrence: SceneCacheOccurrence::Member(SceneMemberId::new_v4()),
            },
            payload_key: ScenePayloadKey {
                scene_id: scene,
                blob_hash: HashDigest::new(digest),
            },
            transform: ScenePlacementTransform::IDENTITY,
            bounds: Bounds3 {
                min: [-1.0; 3],
                max: [1.0; 3],
            },
            cpu_bytes: 1,
            gpu_bytes: 1,
        });
    }
    let keys = payloads.iter().map(|payload| payload.payload_key).collect::<Vec<_>>();
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, 51, payloads);
    let sample = CameraSample::default();
    assert!(authority.update_camera_near(camera_admission(sample)));
    assert_eq!(authority.queue_len(), DEFAULT_LOADER_CAPACITY);
    while let Some(job) = authority.begin_next_load() {
        assert!(authority.reject_load(&job));
    }
    assert_eq!(authority.queue_len(), 0);
    assert!(keys.iter().any(|key| {
        authority.state(key) == Some(PayloadResidencyState::Unloaded)
    }));

    assert!(authority.update_camera_near(camera_admission(sample)));
    assert_eq!(authority.queue_len(), DEFAULT_LOADER_CAPACITY);
    assert_eq!(authority.camera_retry_keys.len(), 1);
    while let Some(job) = authority.begin_next_load() {
        assert!(authority.complete_cpu(job.key, job.generation, 1, 1));
    }
    assert!(authority.update_camera_near(camera_admission(sample)));
    while let Some(job) = authority.begin_next_load() {
        assert!(authority.complete_cpu(job.key, job.generation, 1, 1));
    }
    assert!(authority.camera_retry_keys.is_empty());
    for _ in 0..8 {
        assert!(!authority.update_camera_near(camera_admission(sample)));
    }

    authority.install_scene(scene, 52, Vec::new());
    assert!(!authority.request_reason(
        keys[0],
        ResidencyReason::CameraNear,
        51,
        1,
        1,
    ));
}
