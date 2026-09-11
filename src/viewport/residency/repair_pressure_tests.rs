use std::path::PathBuf;

use bevy::ecs::system::SystemState;
use bevy::prelude::{Res, ResMut, World};
use usd_model::HashDigest;
use usd_project::SceneId;

use crate::project::cache_contract::SceneCacheDescriptorV3;
use crate::project::cache_demand_projection::TargetedPersistenceOutcome;

use super::super::authority::DEFAULT_LOADER_CAPACITY;
use super::super::loader::LoadJob;
use super::super::repair_persistence::{
    TargetedRepairPersistenceCompletion, TargetedRepairPersistenceWorker,
};
use super::super::repair_phase::RepairPhase;
use super::super::{
    PayloadResidencyState, ResidencyAuthority, ResidencyReason, ScenePayloadKey,
};
use super::{
    TARGETED_REPAIR_QUEUE_CAPACITY, TargetedRepairQueue, TargetedRepairRequest,
    drain_targeted_repair_persistence_completions,
};
use super::repair_handoff::{enqueue_lookup_repair, retry_cached_load};

fn job(scene: SceneId, value: u8, generation: u64) -> LoadJob<ScenePayloadKey> {
    LoadJob {
        key: ScenePayloadKey {
            scene_id: scene,
            blob_hash: HashDigest::new([value; HashDigest::BYTE_LEN]),
        },
        generation,
        cpu_bytes: 1,
        gpu_bytes: 1,
    }
}

fn distinct_job(scene: SceneId, value: u8, generation: u64) -> LoadJob<ScenePayloadKey> {
    let mut bytes = [1; HashDigest::BYTE_LEN];
    bytes[0] = value;
    LoadJob {
        key: ScenePayloadKey {
            scene_id: scene,
            blob_hash: HashDigest::new(bytes),
        },
        generation,
        cpu_bytes: 1,
        gpu_bytes: 1,
    }
}

fn request(job: LoadJob<ScenePayloadKey>) -> TargetedRepairRequest {
    TargetedRepairRequest {
        project_root: Some(PathBuf::from("/tmp/c7-pressure")),
        path: Some("/SceneRoot/Owned".to_owned()),
        extraction: None,
        lookup_only: false,
        phase: RepairPhase::NeedScenePayload,
        job,
    }
}

fn dual_pressure_completion(outcome: TargetedPersistenceOutcome) {
    let scene = SceneId::new_v4();
    let generation = 61;
    let target = job(scene, 240, generation);
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, generation, Vec::new());
    assert!(authority.request_reason(
        target.key,
        ResidencyReason::CameraNear,
        generation,
        target.cpu_bytes,
        target.gpu_bytes,
    ));
    let target = authority.begin_next_load().expect("target starts Loading");
    assert!(authority.wait_for_repair(&target));

    let mut repairs = TargetedRepairQueue::default();
    for value in 0..TARGETED_REPAIR_QUEUE_CAPACITY as u8 {
        assert!(repairs.enqueue(request(job(scene, value, generation))));
    }
    for value in 0..DEFAULT_LOADER_CAPACITY as u16 {
        let load = distinct_job(scene, value as u8, generation);
        assert!(authority.request_reason(
            load.key,
            ResidencyReason::CameraNear,
            generation,
            load.cpu_bytes,
            load.gpu_bytes,
        ));
    }

    let completion = TargetedRepairPersistenceCompletion {
        job: target.clone(),
        project_root: PathBuf::from("/tmp/c7-pressure"),
        path: "/SceneRoot/Owned".to_owned(),
        lookup_only: false,
        phase: RepairPhase::NeedScenePayload,
        result: Ok(outcome),
    };
    let worker = TargetedRepairPersistenceWorker::with_completion_for_test(completion);
    let mut world = World::new();
    world.insert_resource(worker);
    world.insert_resource(authority);
    world.insert_resource(repairs);
    let mut state: SystemState<(
        Res<TargetedRepairPersistenceWorker>,
        ResMut<ResidencyAuthority>,
        ResMut<TargetedRepairQueue>,
    )> = SystemState::new(&mut world);
    {
        let (worker, authority, repairs) = state
            .get_mut(&mut world)
            .expect("dual-pressure resources are present");
        drain_targeted_repair_persistence_completions(worker, authority, repairs);
    }
    state.apply(&mut world);

    let authority = world.resource::<ResidencyAuthority>();
    assert_eq!(
        authority.state(&target.key),
        Some(PayloadResidencyState::Unloaded)
    );
    assert_eq!(authority.accounted_bytes(), (0, 0));
    drop(authority);
    let mut authority = world.resource_mut::<ResidencyAuthority>();
    let _ = authority.begin_next_load();
    assert!(authority.request_reason(
        target.key,
        ResidencyReason::CameraNear,
        generation,
        target.cpu_bytes,
        target.gpu_bytes,
    ));
    assert_eq!(
        authority.state(&target.key),
        Some(PayloadResidencyState::Queued)
    );
}

#[test]
fn cas_lost_under_dual_pressure_rejects_without_stranding_retry() {
    dual_pressure_completion(TargetedPersistenceOutcome::CasLost);
}

#[test]
fn waiting_under_dual_pressure_rejects_without_stranding_retry() {
    dual_pressure_completion(TargetedPersistenceOutcome::Waiting);
}

#[test]
fn missing_scene_payload_under_dual_pressure_requeues_targeted_retry() {
    dual_pressure_completion(TargetedPersistenceOutcome::ScenePayloadMissing);
}

#[test]
fn stale_dual_pressure_completion_does_not_resurrect_old_generation() {
    let scene = SceneId::new_v4();
    let old_generation = 62;
    let target = job(scene, 241, old_generation);
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, old_generation, Vec::new());
    assert!(authority.request_reason(
        target.key,
        ResidencyReason::CameraNear,
        old_generation,
        target.cpu_bytes,
        target.gpu_bytes,
    ));
    let target = authority.begin_next_load().expect("target starts Loading");
    assert!(authority.wait_for_repair(&target));
    let worker = TargetedRepairPersistenceWorker::with_completion_for_test(
        TargetedRepairPersistenceCompletion {
            job: target.clone(),
            project_root: PathBuf::from("/tmp/c7-pressure"),
            path: "/SceneRoot/Owned".to_owned(),
            lookup_only: false,
            phase: RepairPhase::NeedScenePayload,
            result: Ok(TargetedPersistenceOutcome::CasLost),
        },
    );
    authority.install_scene(scene, old_generation + 1, Vec::new());
    let mut world = World::new();
    world.insert_resource(worker);
    world.insert_resource(authority);
    world.insert_resource(TargetedRepairQueue::default());
    let mut state: SystemState<(
        Res<TargetedRepairPersistenceWorker>,
        ResMut<ResidencyAuthority>,
        ResMut<TargetedRepairQueue>,
    )> = SystemState::new(&mut world);
    {
        let (worker, authority, repairs) = state
            .get_mut(&mut world)
            .expect("stale completion resources are present");
        drain_targeted_repair_persistence_completions(worker, authority, repairs);
    }
    state.apply(&mut world);
    assert_eq!(
        world.resource::<ResidencyAuthority>().state(&target.key),
        None
    );
    assert_eq!(world.resource::<TargetedRepairQueue>().len(), 0);
}

#[test]
fn lookup_only_pressure_preserves_phase_for_cached_retry() {
    let scene = SceneId::new_v4();
    let generation = 63;
    let target = job(scene, 242, generation);
    let expected = SceneCacheDescriptorV3::invalidated(
        scene,
        generation,
        HashDigest::new([3; HashDigest::BYTE_LEN]),
    );
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, generation, Vec::new());
    assert!(authority.request_reason(
        target.key,
        ResidencyReason::CameraNear,
        generation,
        target.cpu_bytes,
        target.gpu_bytes,
    ));
    let target = authority.begin_next_load().expect("lookup retry starts Loading");
    assert!(authority.wait_for_repair(&target));
    let mut repairs = TargetedRepairQueue::default();
    for value in 0..TARGETED_REPAIR_QUEUE_CAPACITY as u8 {
        assert!(repairs.enqueue(request(job(scene, value, generation))));
    }
    enqueue_lookup_repair(
        &mut authority,
        &mut repairs,
        target.clone(),
        PathBuf::from("/tmp/c7-phase-pressure"),
        "/SceneRoot/Owned".to_owned(),
        expected.clone(),
    );
    let retry = authority.begin_next_load().expect("phase-preserving loader retry");
    repairs.clear();
    super::enqueue_missing_payload_repair(None, &mut authority, &mut repairs, retry);
    let request = repairs.take_front().expect("lookup phase re-enters repair queue");
    assert!(request.lookup_only);
    assert!(request.extraction.as_ref().is_some_and(|repair| {
        repair.payloads.is_empty() && repair.expected_descriptor == expected
    }));
    assert!(matches!(
        request.phase,
        RepairPhase::NeedProjectLookup { .. }
    ));
}

#[test]
fn cache_ready_pressure_never_restarts_scene_payload_repair() {
    let scene = SceneId::new_v4();
    let generation = 64;
    let target = job(scene, 243, generation);
    let expected = SceneCacheDescriptorV3::invalidated(
        scene,
        generation,
        HashDigest::new([4; HashDigest::BYTE_LEN]),
    );
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, generation, Vec::new());
    assert!(authority.request_reason(
        target.key,
        ResidencyReason::CameraNear,
        generation,
        target.cpu_bytes,
        target.gpu_bytes,
    ));
    let target = authority.begin_next_load().expect("cache-ready retry starts Loading");
    assert!(authority.wait_for_repair(&target));
    let mut repairs = TargetedRepairQueue::default();
    retry_cached_load(
        &mut authority,
        &mut repairs,
        target,
        PathBuf::from("/tmp/c7-phase-pressure"),
        "/SceneRoot/Owned".to_owned(),
        expected.clone(),
    );
    let retry = authority.begin_next_load().expect("cache-ready loader retry");
    super::enqueue_missing_payload_repair(None, &mut authority, &mut repairs, retry);
    let request = repairs.take_front().expect("cache-ready phase re-enters repair queue");
    assert!(request.lookup_only);
    assert!(request.extraction.as_ref().is_some_and(|repair| {
        repair.payloads.is_empty() && repair.expected_descriptor == expected
    }));
    assert!(matches!(request.phase, RepairPhase::NeedProjectLookup { .. }));
}
