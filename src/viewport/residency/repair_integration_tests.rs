use std::path::Path;
use std::thread;
use std::time::Duration;
use anyhow::Result;
use bevy::asset::{Assets, RenderAssetUsages};
use bevy::ecs::system::SystemState;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::prelude::{NonSend, Res, ResMut, World};
use usd_model::HashDigest;
use usd_project::{
    ProjectId, ProjectManifestV1, ProjectRoot, SceneId, SceneManifestEntry, StorageKey,
};
use viewport_protocol::RuntimeProfile;
use crate::project::blob_store::prepare_mesh_payload;
use crate::project::cache::{ProjectCacheIdentity, ProjectCacheTarget, SceneCacheStore};
use crate::project::cache_contract::{
    CachedTransform, SCENE_CACHE_INDEX_SCHEMA_VERSION, SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
    SceneCacheAddress, SceneCacheDescriptorV3, SceneCacheEntry, SceneCacheEntryKind,
    SceneCacheIndex, SceneCacheOccurrence, SceneCacheState, SceneSpatialIndex,
};
use crate::project::cache_demand_projection::{
    TargetedPersistenceOutcome, TargetedSceneRepair,
};
use crate::project::cache_hydration::ActiveProjectCacheContext;
use crate::project::catalog::manifest_store::ManifestStore;
use usd_bevy::UsdSnippet;
use super::super::loader::LoadJob;
use super::super::repair_persistence::{
    TargetedRepairPersistenceCompletion, TargetedRepairPersistenceWorker,
};
use super::super::repair_phase::RepairPhase;
use super::super::worker::CachedResidencyWorker;
use super::super::{
    PayloadResidencyState, ResidencyAuthority, ResidencyReason, ScenePayloadKey,
};
use super::{
    TargetedRepairQueue, TargetedRepairRequest, drain_cached_residency_completions,
    drain_targeted_repair_persistence_completions, process_targeted_residency_repairs,
};
fn mesh() -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
    mesh
}
fn live_stage(path: &str) -> Result<usd_bevy::LiveStage> {
    let source = format!(
        "#usda 1.0\n\ndef Xform \"SceneRoot\" {{\n    def Mesh \"{}\" {{\n        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]\n        int[] faceVertexCounts = [3]\n        int[] faceVertexIndices = [0, 1, 2]\n    }}\n}}\n",
        path.rsplit('/').next().unwrap_or("Owned")
    );
    Ok(usd_bevy::LiveStage::new(UsdSnippet::new(source).open_stage()?))
}
fn hash_for(mesh: &Mesh) -> HashDigest {
    let prepared = prepare_mesh_payload(mesh).expect("test mesh prepares");
    HashDigest::from_hex(&prepared.blob_id.0).expect("prepared blob hash decodes")
}
fn owned_entry(scene: SceneId, hash: HashDigest, path: &str) -> SceneCacheEntry {
    SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id: scene,
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
        content_hash: Some(hash),
    }
}
fn owner_index(scene: SceneId, generation: u64, hash: HashDigest, path: &str) -> SceneCacheIndex {
    SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id: scene,
        generation,
        entries: vec![owned_entry(scene, hash, path)],
    }
}
fn owner_index_for_entries(
    scene: SceneId,
    generation: u64,
    entries: Vec<(HashDigest, &str)>,
) -> SceneCacheIndex {
    let mut entries = entries
        .into_iter()
        .map(|(hash, path)| owned_entry(scene, hash, path))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.address.cmp(&right.address));
    SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id: scene,
        generation,
        entries,
    }
}
fn publish_owner_cache(
    root: &Path,
    scene: SceneId,
    generation: u64,
    hash: HashDigest,
    path: &str,
    state: SceneCacheState,
) -> Result<SceneCacheDescriptorV3> {
    let mut descriptor = SceneCacheDescriptorV3::invalidated(
        scene,
        generation,
        HashDigest::new([17; HashDigest::BYTE_LEN]),
    );
    descriptor.state = state;
    let index = owner_index(scene, generation, hash, path);
    let spatial = SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id: scene,
        generation,
        entries: Vec::new(),
    };
    Ok(SceneCacheStore::new(root).publish_generation(&descriptor, &index, &spatial)?)
}
fn write_manifest(root: &Path, scenes: &[SceneId]) -> Result<()> {
    let entries = scenes
        .iter()
        .enumerate()
        .map(|(index, id)| {
            Ok(SceneManifestEntry {
                id: *id,
                storage_key: StorageKey::new(format!("scene-{index}"))?,
                display_name: format!("Scene {index}"),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "C7 targeted repair",
        ProjectRoot::Scene(scenes[0]),
        entries,
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(root, &manifest)
}
fn context(root: &Path) -> ActiveProjectCacheContext {
    ActiveProjectCacheContext::from_identity(
        root.to_path_buf(),
        ProjectCacheIdentity {
            target: ProjectCacheTarget::ProjectRoot,
            target_content_hash: HashDigest::new([18; HashDigest::BYTE_LEN]),
            profile: RuntimeProfile::NativeMedium,
            config_hash: HashDigest::new([19; HashDigest::BYTE_LEN]),
        },
    )
}
fn load_job(scene: SceneId, hash: HashDigest, generation: u64) -> LoadJob<ScenePayloadKey> {
    LoadJob {
        key: ScenePayloadKey {
            scene_id: scene,
            blob_hash: hash,
        },
        generation,
        cpu_bytes: 8,
        gpu_bytes: 8,
    }
}
fn waiting_authority(job: &LoadJob<ScenePayloadKey>) -> ResidencyAuthority {
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(job.key.scene_id, job.generation, Vec::new());
    assert!(authority.request_reason(
        job.key,
        ResidencyReason::CameraNear,
        job.generation,
        job.cpu_bytes,
        job.gpu_bytes,
    ));
    let loading = authority.begin_next_load().expect("repair job starts");
    assert!(authority.wait_for_repair(&loading));
    authority
}
fn repair_request(root: &Path, job: LoadJob<ScenePayloadKey>) -> TargetedRepairRequest {
    TargetedRepairRequest {
        project_root: Some(root.to_path_buf()),
        path: None,
        extraction: None,
        lookup_only: false,
        phase: RepairPhase::NeedScenePayload,
        job,
    }
}
fn wait_persistence(
    worker: &TargetedRepairPersistenceWorker,
) -> TargetedRepairPersistenceCompletion {
    for _ in 0..512 {
        if let Some(completion) = worker.drain_completions().into_iter().next() {
            return completion;
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("bounded repair persistence worker did not complete");
}
fn process_once(world: &mut World) {
    let mut state: SystemState<(
        Option<Res<ActiveProjectCacheContext>>,
        Res<TargetedRepairPersistenceWorker>,
        ResMut<ResidencyAuthority>,
        ResMut<TargetedRepairQueue>,
        Option<NonSend<usd_bevy::LiveStage>>,
    )> = SystemState::new(world);
    {
        let (context, persistence, authority, repairs, live) =
            state.get_mut(world).expect("repair process resources are present");
        process_targeted_residency_repairs(context, persistence, authority, repairs, live);
    }
    state.apply(world);
}
fn drain_persistence_once(world: &mut World) {
    let mut state: SystemState<(
        Res<TargetedRepairPersistenceWorker>,
        ResMut<ResidencyAuthority>,
        ResMut<TargetedRepairQueue>,
    )> = SystemState::new(world);
    {
        let (worker, authority, repairs) = state.get_mut(world).expect("persistence resources");
        drain_targeted_repair_persistence_completions(worker, authority, repairs);
    }
    state.apply(world);
}
fn dispatch_cached_once(world: &mut World) {
    let mut state: SystemState<(
        Option<Res<ActiveProjectCacheContext>>,
        Res<CachedResidencyWorker>,
        ResMut<ResidencyAuthority>,
    )> = SystemState::new(world);
    {
        let (context, worker, authority) = state.get_mut(world).expect("cached resources");
        super::super::dispatch_cached_residency_loads(context, worker, authority);
    }
    state.apply(world);
}
fn drain_cached_once(world: &mut World) {
    let mut state: SystemState<(
        Option<Res<ActiveProjectCacheContext>>,
        Res<CachedResidencyWorker>,
        ResMut<ResidencyAuthority>,
        ResMut<TargetedRepairQueue>,
    )> = SystemState::new(world);
    {
        let (context, worker, authority, repairs) = state.get_mut(world).expect("cached drains");
        drain_cached_residency_completions(context, worker, authority, repairs);
    }
    state.apply(world);
}
#[test]
fn waiting_key_rotates_while_ready_key_reaches_real_persistence() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let scene = SceneId::new_v4();
    let ready_mesh = mesh();
    let ready_hash = hash_for(&ready_mesh);
    let waiting_hash = HashDigest::new([20; HashDigest::BYTE_LEN]);
    let index = owner_index_for_entries(
        scene,
        1,
        vec![(waiting_hash, "/SceneRoot/Waiting"), (ready_hash, "/SceneRoot/Ready")],
    );
    let mut descriptor = SceneCacheDescriptorV3::invalidated(
        scene,
        1,
        HashDigest::new([17; HashDigest::BYTE_LEN]),
    );
    descriptor.state = SceneCacheState::Partial;
    let spatial = SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id: scene,
        generation: 1,
        entries: Vec::new(),
    };
    let expected = SceneCacheStore::new(directory.path())
        .publish_generation(&descriptor, &index, &spatial)?;
    let waiting = load_job(scene, waiting_hash, 1);
    let ready = load_job(scene, ready_hash, 1);
    let mut authority = waiting_authority(&waiting);
    assert!(authority.request_reason(
        ready.key,
        ResidencyReason::CameraNear,
        1,
        ready.cpu_bytes,
        ready.gpu_bytes,
    ));
    let ready_loading = authority.begin_next_load().expect("ready job starts");
    assert!(authority.wait_for_repair(&ready_loading));
    let mut repairs = TargetedRepairQueue::default();
    assert!(repairs.enqueue(repair_request(directory.path(), waiting)));
    assert!(repairs.enqueue(TargetedRepairRequest {
        project_root: Some(directory.path().to_path_buf()),
        path: Some("/SceneRoot/Ready".to_owned()),
        extraction: Some(TargetedSceneRepair {
            expected_descriptor: expected,
            payloads: vec![usd_bevy::TargetedRenderPayload {
                path: "/SceneRoot/Ready".to_owned(),
                mesh: ready_mesh,
                local_bounds: None,
            }],
        }),
        lookup_only: false,
        phase: RepairPhase::NeedScenePayload,
        job: ready_loading.clone(),
    }));

    let mut world = World::new();
    world.insert_resource(context(directory.path()));
    world.insert_resource(TargetedRepairPersistenceWorker::new());
    world.insert_resource(authority);
    world.insert_resource(repairs);
    process_once(&mut world);
    assert_eq!(world.resource::<TargetedRepairQueue>().len(), 1);
    let completion = wait_persistence(world.resource::<TargetedRepairPersistenceWorker>());
    assert_eq!(completion.job.key, ready.key);
    assert!(matches!(
        completion.result,
        Ok(TargetedPersistenceOutcome::Published { .. })
    ));
    Ok(())
}
#[test]
fn real_repair_pipeline_reaches_gpu_residency() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let scene = SceneId::new_v4();
    let live = live_stage("Owned")?;
    let payload = usd_bevy::extract_render_payloads_for_paths(&live.stage, &["/SceneRoot/Owned"])?
        .into_iter()
        .next()
        .expect("LiveStage exposes requested mesh");
    let hash = hash_for(&payload.mesh);
    publish_owner_cache(
        directory.path(),
        scene,
        1,
        hash,
        "/SceneRoot/Owned",
        SceneCacheState::Partial,
    )?;
    write_manifest(directory.path(), &[scene])?;
    let job = load_job(scene, hash, 1);
    let mut authority = waiting_authority(&job);
    let mut repairs = TargetedRepairQueue::default();
    assert!(repairs.enqueue(repair_request(directory.path(), job.clone())));

    let mut world = World::new();
    world.insert_resource(context(directory.path()));
    world.insert_resource(TargetedRepairPersistenceWorker::new());
    world.insert_resource(CachedResidencyWorker::new());
    world.insert_resource(authority);
    world.insert_resource(repairs);
    world.insert_non_send(live);
    process_once(&mut world);
    for _ in 0..512 {
        drain_persistence_once(&mut world);
        if world.resource::<ResidencyAuthority>().state(&job.key)
            == Some(PayloadResidencyState::Queued)
        {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        world.resource::<ResidencyAuthority>().state(&job.key),
        Some(PayloadResidencyState::Queued)
    );
    dispatch_cached_once(&mut world);
    for _ in 0..512 {
        drain_cached_once(&mut world);
        if world.resource::<ResidencyAuthority>().state(&job.key)
            == Some(PayloadResidencyState::CpuReady)
        {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        world.resource::<ResidencyAuthority>().state(&job.key),
        Some(PayloadResidencyState::CpuReady)
    );
    let mut assets = Assets::<Mesh>::default();
    let uploaded = world
        .resource_mut::<ResidencyAuthority>()
        .pump_uploads(&mut assets, Some(usize::MAX));
    assert_eq!(uploaded, vec![job.key]);
    assert_eq!(
        world.resource::<ResidencyAuthority>().state(&job.key),
        Some(PayloadResidencyState::GpuResident)
    );
    Ok(())
}
