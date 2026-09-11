use super::*;
use bevy::asset::Assets;
use usd_model::HashDigest;
use usd_project::SceneId;

use super::super::{ResidencyBudgets, ResidencyReason};

fn key(scene_id: SceneId, value: u8) -> ScenePayloadKey {
    ScenePayloadKey {
        scene_id,
        blob_hash: HashDigest::new([value; HashDigest::BYTE_LEN]),
    }
}

#[test]
fn oversized_ready_payload_starts_without_starving_smaller_work() {
    let scene = SceneId::new_v4();
    let oversized = key(scene, 1);
    let smaller = key(scene, 2);
    let mut authority = ResidencyAuthority::with_budgets(ResidencyBudgets {
        cpu_bytes: 32,
        gpu_bytes: 16,
        upload_bytes_per_frame: 4,
    });
    let mut assets = Assets::<Mesh>::default();
    authority.install_scene(scene, 81, Vec::new());
    assert!(authority.request_reason(oversized, ResidencyReason::CameraNear, 81, 8, 8,));
    assert!(authority.request_reason(smaller, ResidencyReason::CameraNear, 81, 1, 1,));
    let oversized_job = authority.begin_next_load().expect("oversized job");
    let smaller_job = authority.begin_next_load().expect("smaller job");
    assert!(authority.complete_cpu(oversized_job.key, 81, 8, 8));
    assert!(authority.complete_cpu(smaller_job.key, 81, 1, 1));

    assert_eq!(
        authority.pump_uploads(&mut assets, Some(4)),
        vec![oversized]
    );
    assert_eq!(
        authority.state(&oversized),
        Some(PayloadResidencyState::GpuResident)
    );
    assert_eq!(
        authority.state(&smaller),
        Some(PayloadResidencyState::CpuReady)
    );
    assert_eq!(authority.pump_uploads(&mut assets, Some(4)), vec![smaller]);
}

#[test]
fn ready_upload_order_stays_deduplicated_through_demand_churn() {
    let scene = SceneId::new_v4();
    let key = key(scene, 3);
    let mut authority = ResidencyAuthority::with_budgets(ResidencyBudgets {
        cpu_bytes: 32,
        gpu_bytes: 32,
        upload_bytes_per_frame: 8,
    });
    let mut assets = Assets::<Mesh>::default();
    authority.install_scene(scene, 82, Vec::new());
    assert!(authority.request_reason(key, ResidencyReason::CameraNear, 82, 8, 8,));
    let job = authority.begin_next_load().expect("queued payload");
    assert!(authority.complete_cpu(job.key, 82, 8, 8));

    for _ in 0..512 {
        assert_eq!(authority.ready_for_upload.len(), 1);
        assert_eq!(authority.ready_upload_membership.len(), 1);
        assert!(authority.remove_reason(key, ResidencyReason::CameraNear, 82));
        assert_eq!(authority.ready_for_upload.len(), 0);
        assert_eq!(authority.ready_upload_membership.len(), 0);
        assert!(authority.request_reason(key, ResidencyReason::CameraNear, 82, 8, 8,));
        assert_eq!(authority.ready_for_upload.len(), 1);
        assert_eq!(authority.ready_upload_membership.len(), 1);
    }

    assert_eq!(authority.pump_uploads(&mut assets, Some(8)), vec![key]);
    assert!(authority.ready_for_upload.is_empty());
    assert!(authority.ready_upload_membership.is_empty());
}
