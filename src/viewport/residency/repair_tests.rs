use super::*;
use crate::project::cache::{ProjectCacheIdentity, ProjectCacheTarget, SceneCacheStore};
use crate::project::cache_contract::{
    CachedTransform, SCENE_CACHE_INDEX_SCHEMA_VERSION, SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
    SceneCacheAddress, SceneCacheDescriptorV3, SceneCacheEntry, SceneCacheEntryKind,
    SceneCacheIndex, SceneCacheOccurrence, SceneSpatialIndex,
};
use crate::project::cache_hydration::ActiveProjectCacheContext;
use bevy::asset::Assets;
use bevy::ecs::system::SystemState;
use bevy::mesh::Mesh;
use bevy::prelude::{Res, ResMut, World};
use usd_model::HashDigest;
use usd_project::SceneId;
use viewport_protocol::RuntimeProfile;
use super::super::repair_phase::RepairPhase;

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

fn request(job: LoadJob<ScenePayloadKey>, path: &str) -> TargetedRepairRequest {
    TargetedRepairRequest {
        project_root: Some(PathBuf::from("/tmp/c7-repair-test")),
        path: Some(path.to_owned()),
        extraction: None,
        lookup_only: false,
        phase: RepairPhase::NeedScenePayload,
        job,
    }
}

#[test]
fn camera_near_repair_retries_same_key_into_cpu_ready() {
    let scene = SceneId::new_v4();
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, 7, Vec::new());
    let key = job(scene, 1, 7).key;
    assert!(authority.request_reason(key, super::super::ResidencyReason::CameraNear, 7, 8, 8));
    let load = authority.begin_next_load().expect("CameraNear worker load");
    let mut repairs = TargetedRepairQueue::default();
    assert!(repairs.enqueue(request(load.clone(), "/SceneRoot/Owned")));
    let repair = repairs.take_front().expect("bounded targeted repair");
    assert!(authority.load_is_current(&repair.job));
    assert!(authority.complete_cached_cpu(load, test_mesh()));
    assert_eq!(
        authority.state(&key),
        Some(super::super::PayloadResidencyState::CpuReady)
    );
}

#[test]
fn cached_worker_none_is_queued_without_inline_targeted_fill() {
    let directory = tempfile::tempdir().expect("repair integration directory");
    let scene = SceneId::new_v4();
    let load = job(scene, 5, 17);
    let index = SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id: scene,
        generation: 17,
        entries: vec![SceneCacheEntry {
            address: SceneCacheAddress {
                scene_id: scene,
                occurrence: SceneCacheOccurrence::PrimPath("/SceneRoot/Owned".to_owned()),
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
                prim_path: "/SceneRoot/Owned".to_owned(),
            },
            content_hash: Some(load.key.blob_hash),
        }],
    };
    let spatial = SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id: scene,
        generation: 17,
        entries: Vec::new(),
    };
    SceneCacheStore::new(directory.path())
        .publish_generation(
            &SceneCacheDescriptorV3::invalidated(
                scene,
                17,
                HashDigest::new([6; HashDigest::BYTE_LEN]),
            ),
            &index,
            &spatial,
        )
        .expect("publish partial owner cache");

    let identity = ProjectCacheIdentity {
        target: ProjectCacheTarget::ProjectRoot,
        target_content_hash: HashDigest::new([7; HashDigest::BYTE_LEN]),
        profile: RuntimeProfile::NativeMedium,
        config_hash: HashDigest::new([8; HashDigest::BYTE_LEN]),
    };
    let context =
        ActiveProjectCacheContext::from_identity(directory.path().to_path_buf(), identity);
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, 17, Vec::new());
    assert!(authority.request_reason(
        load.key,
        super::super::ResidencyReason::CameraNear,
        17,
        load.cpu_bytes,
        load.gpu_bytes,
    ));
    let load = authority.begin_next_load().expect("cached worker load");
    let worker = super::super::worker::CachedResidencyWorker::with_completion_for_test(
        super::super::worker::LoadCompletion {
            job: load.clone(),
            result: Ok(None),
        },
    );
    let mut world = World::new();
    world.insert_resource(context);
    world.insert_resource(worker);
    world.insert_resource(authority);
    world.insert_resource(TargetedRepairQueue::default());
    let mut state: SystemState<(
        Option<Res<ActiveProjectCacheContext>>,
        Res<super::super::worker::CachedResidencyWorker>,
        ResMut<ResidencyAuthority>,
        ResMut<TargetedRepairQueue>,
    )> = SystemState::new(&mut world);
    {
        let (context, worker, authority, repairs) = state
            .get_mut(&mut world)
            .expect("completion-drain resources are present");
        drain_cached_residency_completions(context, worker, authority, repairs);
    }
    state.apply(&mut world);

    assert_eq!(world.resource::<TargetedRepairQueue>().len(), 1);
    assert_eq!(
        world.resource::<ResidencyAuthority>().state(&load.key),
        Some(super::super::PayloadResidencyState::RepairWaiting)
    );
}

#[test]
fn repair_burst_is_bounded_and_coalesces_duplicate_keys() {
    let scene = SceneId::new_v4();
    let mut repairs = TargetedRepairQueue::default();
    for value in 0..TARGETED_REPAIR_QUEUE_CAPACITY as u8 {
        let load = job(scene, value, 9);
        assert!(repairs.enqueue(request(load, "/SceneRoot/Owned")));
    }
    let duplicate = job(scene, 0, 9);
    assert!(repairs.enqueue(request(duplicate, "/SceneRoot/Owned/Latest")));
    assert_eq!(repairs.len(), TARGETED_REPAIR_QUEUE_CAPACITY);
    assert!(!repairs.enqueue(request(job(scene, 99, 9), "/SceneRoot/Overflow")));
}

#[test]
fn full_repair_queue_defers_the_loading_job_without_losing_camera_retry() {
    let scene = SceneId::new_v4();
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, 9, Vec::new());
    let mut repairs = TargetedRepairQueue::default();
    for value in 0..TARGETED_REPAIR_QUEUE_CAPACITY as u8 {
        assert!(repairs.enqueue(request(job(scene, value, 9), "/SceneRoot/Busy")));
    }
    let overflow = job(scene, 99, 9);
    assert!(authority.request_reason(
        overflow.key,
        super::super::ResidencyReason::CameraNear,
        9,
        overflow.cpu_bytes,
        overflow.gpu_bytes,
    ));
    let overflow = authority.begin_next_load().expect("overflow worker load");
    let retry = overflow.clone();
    assert!(!repairs.enqueue(request(overflow, "/SceneRoot/Overflow")));
    assert!(authority.defer_load(retry.clone()));
    assert_eq!(
        authority.state(&retry.key),
        Some(super::super::PayloadResidencyState::Queued)
    );
    assert!(
        authority
            .reasons(&retry.key)
            .unwrap()
            .contains(&super::super::ResidencyReason::CameraNear)
    );
}

#[test]
fn stale_generation_is_dropped_before_repair_work() {
    let scene = SceneId::new_v4();
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, 10, Vec::new());
    let load = {
        let key = job(scene, 3, 10).key;
        assert!(
            authority.request_reason(key, super::super::ResidencyReason::CameraNear, 10, 8, 8,)
        );
        authority.begin_next_load().expect("current load")
    };
    let mut repairs = TargetedRepairQueue::default();
    assert!(repairs.enqueue(request(load, "/SceneRoot/Stale")));
    authority.install_scene(scene, 11, Vec::new());
    let stale = repairs.take_front().expect("stale queued repair");
    assert!(!authority.load_is_current(&stale.job));
}

#[test]
fn worker_retry_keeps_upload_throttle_owned_by_residency_authority() {
    let scene = SceneId::new_v4();
    let key = job(scene, 4, 12).key;
    let mut authority = ResidencyAuthority::with_budgets(super::super::ResidencyBudgets {
        cpu_bytes: 32,
        gpu_bytes: 32,
        upload_bytes_per_frame: 1,
    });
    authority.install_scene(scene, 12, Vec::new());
    assert!(authority.request_reason(key, super::super::ResidencyReason::CameraNear, 12, 8, 8,));
    let load = authority.begin_next_load().expect("load starts");
    assert!(authority.complete_cpu(load.key, load.generation, load.cpu_bytes, load.gpu_bytes));
    assert_eq!(
        authority.state(&key),
        Some(super::super::PayloadResidencyState::CpuReady)
    );
    let mut assets = Assets::<Mesh>::default();
    let _ = authority.pump_uploads(&mut assets, Some(1));
    assert_eq!(
        authority.state(&key),
        Some(super::super::PayloadResidencyState::GpuResident)
    );
}

fn test_mesh() -> Mesh {
    Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    )
}
