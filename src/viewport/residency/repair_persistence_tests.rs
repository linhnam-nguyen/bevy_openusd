use std::path::Path;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use usd_model::HashDigest;
use usd_project::SceneId;

use crate::project::blob_store::{BlobStore, prepare_mesh_payload};
use crate::project::cache::SceneCacheStore;
use crate::project::cache_contract::{
    CachedTransform, SCENE_CACHE_INDEX_SCHEMA_VERSION, SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
    SceneCacheAddress, SceneCacheDescriptorV3, SceneCacheEntry, SceneCacheEntryKind,
    SceneCacheIndex, SceneCacheOccurrence, SceneCacheState, SceneSpatialIndex,
};
use crate::project::cache_demand_projection::{
    TargetedPersistenceOutcome, TargetedSceneRepair,
};
use crate::project::catalog::manifest_store::ManifestStore;
use usd_project::{ProjectId, ProjectManifestV1, ProjectRoot, SceneManifestEntry, StorageKey};

use super::super::loader::LoadJob;
use super::super::repair_persistence::{
    TargetedRepairPersistenceCompletion, TargetedRepairPersistenceRequest,
    TargetedRepairPersistenceWorker,
};
use super::super::repair_phase::RepairPhase;
use super::super::ScenePayloadKey;

fn mesh() -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
    mesh
}

fn wait(worker: &TargetedRepairPersistenceWorker) -> TargetedRepairPersistenceCompletion {
    for _ in 0..512 {
        if let Some(completion) = worker.drain_completions().into_iter().next() {
            return completion;
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("targeted persistence worker did not complete");
}

fn owner_index(scene: SceneId, hash: HashDigest, path: &str) -> SceneCacheIndex {
    SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id: scene,
        generation: 1,
        entries: vec![SceneCacheEntry {
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
        }],
    }
}

fn write_manifest(root: &Path, scene: SceneId) -> Result<()> {
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "targeted repair lookup",
        ProjectRoot::Scene(scene),
        vec![SceneManifestEntry {
            id: scene,
            storage_key: StorageKey::new("scene")?,
            display_name: "Scene".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(root, &manifest)
}

#[test]
fn published_descriptor_is_carried_into_pure_lookup_retry() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let scene = SceneId::new_v4();
    let path = "/SceneRoot/Owned";
    let source_mesh = mesh();
    let prepared = prepare_mesh_payload(&source_mesh)?;
    let hash = HashDigest::from_hex(&prepared.blob_id.0)?;
    let index = owner_index(scene, hash, path);
    let spatial = SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id: scene,
        generation: 1,
        entries: Vec::new(),
    };
    let mut input = SceneCacheDescriptorV3::invalidated(
        scene,
        1,
        HashDigest::new([31; HashDigest::BYTE_LEN]),
    );
    input.state = SceneCacheState::Partial;
    let expected = SceneCacheStore::new(directory.path()).publish_generation(&input, &index, &spatial)?;
    let job = LoadJob {
        key: ScenePayloadKey {
            scene_id: scene,
            blob_hash: hash,
        },
        generation: 1,
        cpu_bytes: 1,
        gpu_bytes: 1,
    };
    let worker = TargetedRepairPersistenceWorker::new();
    worker.dispatch(TargetedRepairPersistenceRequest {
        project_root: directory.path().to_path_buf(),
        path: path.to_owned(),
        extraction: TargetedSceneRepair {
            expected_descriptor: expected,
            payloads: vec![usd_bevy::TargetedRenderPayload {
                path: path.to_owned(),
                mesh: source_mesh,
                local_bounds: None,
            }],
        },
        lookup_only: false,
        phase: RepairPhase::NeedScenePayload,
        job: job.clone(),
    }).expect("full repair enters bounded persistence");
    let published_completion = wait(&worker);
    let published = match &published_completion.result {
        Ok(TargetedPersistenceOutcome::Published {
            published_descriptor,
            ..
        }) => published_descriptor.clone(),
        other => panic!("post-CAS descriptor is not carried in {other:?}"),
    };
    assert!(matches!(
        &published_completion.result,
        Ok(TargetedPersistenceOutcome::Published {
            lookup_repaired: false,
            published_descriptor,
        }) if published_descriptor == &published
    ));
    let objects = SceneCacheStore::new(directory.path()).object_store(scene)?;
    let object_before = objects
        .get(&usd_model::BlobId(hash.to_string()))?
        .expect("full repair wrote geometry once");

    write_manifest(directory.path(), scene)?;
    worker.dispatch(TargetedRepairPersistenceRequest {
        project_root: directory.path().to_path_buf(),
        path: path.to_owned(),
        extraction: TargetedSceneRepair {
            expected_descriptor: published.clone(),
            payloads: Vec::new(),
        },
        lookup_only: true,
        phase: RepairPhase::NeedProjectLookup {
            project_root: directory.path().to_path_buf(),
            path: path.to_owned(),
            expected_descriptor: published.clone(),
        },
        job: job.clone(),
    }).expect("lookup-only retry enters bounded persistence");
    assert!(matches!(
        wait(&worker).result,
        Ok(TargetedPersistenceOutcome::LookupRepaired)
    ));
    assert_eq!(
        SceneCacheStore::new(directory.path())
            .load_activation(scene)?
            .expect("Scene publication remains current")
            .descriptor,
        published
    );
    assert_eq!(
        objects.get(&usd_model::BlobId(hash.to_string()))?.as_deref(),
        Some(object_before.as_slice())
    );

    let mut newer_input = published.clone();
    newer_input.config_hash = HashDigest::new([32; HashDigest::BYTE_LEN]);
    let store = SceneCacheStore::new(directory.path());
    let newer = store.publish_generation(&newer_input, &index, &spatial)?;
    worker.dispatch(TargetedRepairPersistenceRequest {
        project_root: directory.path().to_path_buf(),
        path: path.to_owned(),
        extraction: TargetedSceneRepair {
            expected_descriptor: published.clone(),
            payloads: Vec::new(),
        },
        lookup_only: true,
        phase: RepairPhase::NeedProjectLookup {
            project_root: directory.path().to_path_buf(),
            path: path.to_owned(),
            expected_descriptor: published.clone(),
        },
        job: job.clone(),
    }).expect("stale lookup enters bounded persistence");
    assert!(matches!(wait(&worker).result, Ok(TargetedPersistenceOutcome::CasLost)));
    worker.dispatch(TargetedRepairPersistenceRequest {
        project_root: directory.path().to_path_buf(),
        path: path.to_owned(),
        extraction: TargetedSceneRepair {
            expected_descriptor: newer.clone(),
            payloads: Vec::new(),
        },
        lookup_only: true,
        phase: RepairPhase::NeedProjectLookup {
            project_root: directory.path().to_path_buf(),
            path: path.to_owned(),
            expected_descriptor: newer.clone(),
        },
        job,
    }).expect("current lookup enters bounded persistence");
    assert!(matches!(
        wait(&worker).result,
        Ok(TargetedPersistenceOutcome::ScenePayloadMissing)
    ));
    Ok(())
}
