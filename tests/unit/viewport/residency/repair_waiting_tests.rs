use std::path::PathBuf;

use bevy::ecs::system::SystemState;
use bevy::mesh::Mesh;
use bevy::prelude::{Res, ResMut, World};
use usd_model::HashDigest;
use usd_project::SceneId;

use crate::project::cache::SceneCacheStore;
use crate::project::cache_contract::{
    CachedTransform, SCENE_CACHE_INDEX_SCHEMA_VERSION, SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
    SceneCacheAddress, SceneCacheDescriptorV3, SceneCacheEntry, SceneCacheEntryKind,
    SceneCacheIndex, SceneCacheOccurrence, SceneCacheState, SceneSpatialIndex,
};
use crate::project::cache_demand_projection::{
    OwnedPrimPathResolution, TargetedPersistenceOutcome, owned_prim_path_for_payload,
};

use super::super::loader::LoadJob;
use super::super::repair_persistence::{
    TargetedRepairPersistenceCompletion, TargetedRepairPersistenceWorker,
};
use super::super::repair_phase::RepairPhase;
use super::super::{
    PayloadResidencyState, ResidencyAuthority, ResidencyBudgets, ResidencyReason, ScenePayloadKey,
};
use super::{
    TARGETED_REPAIR_QUEUE_CAPACITY, TargetedRepairQueue, TargetedRepairRequest,
    drain_targeted_repair_persistence_completions,
};

fn job(scene_id: SceneId, value: u8, generation: u64) -> LoadJob<ScenePayloadKey> {
    LoadJob {
        key: ScenePayloadKey {
            scene_id,
            blob_hash: HashDigest::new([value; HashDigest::BYTE_LEN]),
        },
        generation,
        cpu_bytes: 8,
        gpu_bytes: 8,
    }
}

#[test]
fn repair_waiting_releases_loading_capacity_and_reenters_bounded_admission() {
    let scene = SceneId::new_v4();
    let waiting = job(scene, 10, 21);
    let cached = job(scene, 11, 21);
    let mut authority = ResidencyAuthority::with_budgets(ResidencyBudgets {
        cpu_bytes: 8,
        gpu_bytes: 8,
        upload_bytes_per_frame: 8,
    });
    authority.install_scene(scene, 21, Vec::new());
    assert!(authority.request_reason(waiting.key, ResidencyReason::CameraNear, 21, 8, 8,));
    let waiting = authority.begin_next_load().expect("repair source load");
    assert!(authority.wait_for_repair(&waiting));
    assert_eq!(
        authority.state(&waiting.key),
        Some(PayloadResidencyState::RepairWaiting)
    );
    assert!(authority.request_reason(cached.key, ResidencyReason::CameraNear, 21, 8, 8));
    assert_eq!(authority.begin_next_load().unwrap().key, cached.key);
    assert!(authority.requeue_repair_waiting(&waiting, RepairPhase::NeedScenePayload));
    assert_eq!(
        authority.state(&waiting.key),
        Some(PayloadResidencyState::Queued)
    );
}

#[test]
fn transient_activation_wait_preserves_camera_repair_until_cached_retry() {
    let directory = tempfile::tempdir().expect("transient activation directory");
    let scene = SceneId::new_v4();
    let load = job(scene, 12, 22);
    let index = owner_index(scene, 22, load.key.blob_hash, "/SceneRoot/Owned");
    let spatial = SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id: scene,
        generation: 22,
        entries: Vec::new(),
    };
    let store = SceneCacheStore::new(directory.path());
    let building =
        SceneCacheDescriptorV3::invalidated(scene, 22, HashDigest::new([13; HashDigest::BYTE_LEN]));
    store
        .publish_generation(&building, &index, &spatial)
        .expect("publish Building descriptor");
    assert_eq!(
        owned_prim_path_for_payload(directory.path(), scene, load.key.blob_hash)
            .expect("probe Building activation"),
        OwnedPrimPathResolution::Waiting
    );

    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, 22, Vec::new());
    assert!(authority.request_reason(
        load.key,
        ResidencyReason::CameraNear,
        22,
        load.cpu_bytes,
        load.gpu_bytes,
    ));
    let load = authority.begin_next_load().expect("CameraNear load");
    assert!(authority.wait_for_repair(&load));
    let mut repairs = TargetedRepairQueue::default();
    assert!(repairs.enqueue(TargetedRepairRequest {
        project_root: Some(directory.path().to_path_buf()),
        path: None,
        extraction: None,
        lookup_only: false,
        phase: RepairPhase::NeedScenePayload,
        job: load.clone(),
    }));

    let mut ready = building;
    ready.state = SceneCacheState::Partial;
    store
        .publish_generation(&ready, &index, &spatial)
        .expect("activate same-generation Scene cache");
    assert_eq!(
        owned_prim_path_for_payload(directory.path(), scene, load.key.blob_hash)
            .expect("resolve activated owner"),
        OwnedPrimPathResolution::Ready("/SceneRoot/Owned".to_owned())
    );
    assert!(authority.requeue_repair_waiting(&load, RepairPhase::NeedScenePayload));
    let retry = authority.begin_next_load().expect("same-key cached retry");
    assert!(authority.complete_cached_cpu(retry, mesh()));
    assert_eq!(
        authority.state(&load.key),
        Some(PayloadResidencyState::CpuReady)
    );
    assert_eq!(repairs.len(), 1);
}

#[test]
fn persistence_completion_redispatches_cached_worker_without_loading_leak() {
    let scene = SceneId::new_v4();
    let load = job(scene, 14, 23);
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, 23, Vec::new());
    assert!(authority.request_reason(load.key, ResidencyReason::CameraNear, 23, 8, 8));
    let load = authority.begin_next_load().expect("repair source load");
    assert!(authority.wait_for_repair(&load));
    let completion = TargetedRepairPersistenceCompletion {
        job: load.clone(),
        project_root: PathBuf::from("/tmp/c7-repair-test"),
        path: "/SceneRoot/Owned".to_owned(),
        lookup_only: true,
        phase: RepairPhase::NeedProjectLookup {
            project_root: PathBuf::from("/tmp/c7-repair-test"),
            path: "/SceneRoot/Owned".to_owned(),
            expected_descriptor: SceneCacheDescriptorV3::invalidated(
                scene,
                23,
                HashDigest::new([15; HashDigest::BYTE_LEN]),
            ),
        },
        result: Ok(TargetedPersistenceOutcome::LookupRepaired),
    };
    let worker = TargetedRepairPersistenceWorker::with_completion_for_test(completion);
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
            .expect("persistence completion resources are present");
        drain_targeted_repair_persistence_completions(worker, authority, repairs);
    }
    state.apply(&mut world);
    assert_eq!(
        world.resource::<ResidencyAuthority>().state(&load.key),
        Some(PayloadResidencyState::Queued)
    );
    let mut authority = world.resource_mut::<ResidencyAuthority>();
    let retry = authority
        .begin_next_load()
        .expect("redispatched cached load");
    assert!(authority.complete_cached_cpu(retry, mesh()));
    assert_eq!(
        authority.state(&load.key),
        Some(PayloadResidencyState::CpuReady)
    );
}

#[test]
fn repair_queue_is_bounded_and_coalesces_same_key() {
    let scene = SceneId::new_v4();
    let mut repairs = TargetedRepairQueue::default();
    for value in 0..TARGETED_REPAIR_QUEUE_CAPACITY as u8 {
        let load = job(scene, value, 24);
        assert!(repairs.enqueue(TargetedRepairRequest {
            project_root: None,
            path: None,
            extraction: None,
            lookup_only: false,
            phase: RepairPhase::NeedScenePayload,
            job: load,
        }));
    }
    let duplicate = job(scene, 0, 24);
    assert!(repairs.enqueue(TargetedRepairRequest {
        project_root: None,
        path: Some("/SceneRoot/Latest".to_owned()),
        extraction: None,
        lookup_only: false,
        phase: RepairPhase::NeedScenePayload,
        job: duplicate,
    }));
    assert_eq!(repairs.len(), TARGETED_REPAIR_QUEUE_CAPACITY);
    let overflow = job(scene, 99, 24);
    assert!(!repairs.enqueue(TargetedRepairRequest {
        project_root: None,
        path: None,
        extraction: None,
        lookup_only: false,
        phase: RepairPhase::NeedScenePayload,
        job: overflow,
    }));
}

#[test]
fn retirement_drops_in_flight_repair_completion_and_queued_work() {
    let scene = SceneId::new_v4();
    let load = job(scene, 16, 25);
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, 25, Vec::new());
    assert!(authority.request_reason(load.key, ResidencyReason::CameraNear, 25, 8, 8));
    let load = authority.begin_next_load().expect("repair source load");
    assert!(authority.wait_for_repair(&load));
    let worker = TargetedRepairPersistenceWorker::with_completion_for_test(
        TargetedRepairPersistenceCompletion {
            job: load.clone(),
            project_root: PathBuf::from("/tmp/c7-repair-test"),
            path: "/SceneRoot/Owned".to_owned(),
            lookup_only: true,
            phase: RepairPhase::NeedProjectLookup {
                project_root: PathBuf::from("/tmp/c7-repair-test"),
                path: "/SceneRoot/Owned".to_owned(),
                expected_descriptor: SceneCacheDescriptorV3::invalidated(
                    scene,
                    25,
                    HashDigest::new([17; HashDigest::BYTE_LEN]),
                ),
            },
            result: Ok(TargetedPersistenceOutcome::LookupRepaired),
        },
    );
    let mut repairs = TargetedRepairQueue::default();
    assert!(repairs.enqueue(TargetedRepairRequest {
        project_root: None,
        path: None,
        extraction: None,
        lookup_only: false,
        phase: RepairPhase::NeedScenePayload,
        job: load.clone(),
    }));
    let mut world = World::new();
    world.insert_resource(worker);
    world.insert_resource(authority);
    world.insert_resource(repairs);
    super::super::retire_scene_cache_resources(&mut world);

    let mut state: SystemState<(
        Res<TargetedRepairPersistenceWorker>,
        ResMut<ResidencyAuthority>,
        ResMut<TargetedRepairQueue>,
    )> = SystemState::new(&mut world);
    {
        let (worker, authority, repairs) = state
            .get_mut(&mut world)
            .expect("retirement completion resources are present");
        drain_targeted_repair_persistence_completions(worker, authority, repairs);
    }
    state.apply(&mut world);
    assert_eq!(
        world.resource::<ResidencyAuthority>().state(&load.key),
        None
    );
    assert_eq!(world.resource::<TargetedRepairQueue>().len(), 0);
}

fn owner_index(
    scene_id: SceneId,
    generation: u64,
    content_hash: HashDigest,
    path: &str,
) -> SceneCacheIndex {
    SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation,
        entries: vec![SceneCacheEntry {
            address: SceneCacheAddress {
                scene_id,
                occurrence: SceneCacheOccurrence::PrimPath(path.to_owned()),
            },
            parent: None,
            transform: CachedTransform::Placement(usd_project::ScenePlacementTransform::IDENTITY),
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
            content_hash: Some(content_hash),
        }],
    }
}

fn mesh() -> Mesh {
    Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    )
}
