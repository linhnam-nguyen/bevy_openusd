use super::*;
use bevy::asset::Assets;
use bevy::prelude::{App, Update};
use usd_model::HashDigest;
use usd_project::SceneId;

use crate::project::runtime_payload::{RuntimeAlphaMode, RuntimeMaterialTextures};
use crate::viewport::session::SceneCachePresentation;
use usd_bevy::ProjectionSeed;

fn job(generation: u64) -> LoadJob<ScenePayloadKey> {
    LoadJob {
        key: ScenePayloadKey {
            scene_id: SceneId::new_v4(),
            blob_hash: HashDigest::new([7; HashDigest::BYTE_LEN]),
        },
        generation,
        cpu_bytes: 1,
        gpu_bytes: 1,
    }
}

fn material() -> RuntimeMaterialBlob {
    RuntimeMaterialBlob {
        version: crate::project::runtime_payload::RUNTIME_MATERIAL_VERSION,
        base_color: [1.0; 4],
        emissive: [0.0; 4],
        perceptual_roughness: 1.0,
        metallic: 0.0,
        ior: 1.5,
        alpha_mode: RuntimeAlphaMode::Opaque,
        double_sided: false,
        unlit: false,
        uv_transform: [[1.0, 0.0], [0.0, 1.0], [0.0, 0.0]],
        textures: RuntimeMaterialTextures::default(),
    }
}

#[test]
fn loaded_payloads_are_installed_only_for_the_current_scene_generation() {
    let scene_id = SceneId::new_v4();
    let mut app = App::new();
    app.insert_resource(SceneCachePresentation {
        scene_id,
        generation: 7,
        state: crate::project::cache_contract::SceneCacheState::Ready,
        entries: Vec::new(),
    })
    .insert_resource(Assets::<bevy::image::Image>::default())
    .insert_resource(Assets::<bevy::pbr::StandardMaterial>::default())
    .insert_resource(ProjectionSeed::default())
    .insert_resource(LoadedScenePayloadQueue::default())
    .add_systems(Update, install_loaded_scene_payloads);
    let mut current_job = job(7);
    current_job.key.scene_id = scene_id;
    let stale_job = LoadJob {
        generation: 6,
        ..current_job.clone()
    };
    app.world_mut()
        .resource_mut::<LoadedScenePayloadQueue>()
        .completions
        .extend([
            LoadedScenePayloadCompletion {
                job: stale_job,
                payloads: LoadedScenePayloads {
                    materials: vec![("/World/Stale".to_owned(), material())],
                    textures: Vec::new(),
                    animations: Vec::new(),
                },
            },
            LoadedScenePayloadCompletion {
                job: current_job,
                payloads: LoadedScenePayloads {
                    materials: vec![("/World/Current".to_owned(), material())],
                    textures: Vec::new(),
                    animations: Vec::new(),
                },
            },
        ]);

    app.update();

    assert_eq!(
        app.world().resource::<ProjectionSeed>().pending_materials(),
        1
    );
    assert_eq!(
        app.world()
            .resource::<Assets<bevy::pbr::StandardMaterial>>()
            .len(),
        1
    );
}

#[test]
fn worker_completion_keeps_generation_tagged_and_bounded() {
    let worker = CachedResidencyWorker::new();
    let job = job(17);
    let root = tempfile::tempdir().expect("temporary cache root");
    worker
        .dispatch(root.path().to_path_buf(), job.clone())
        .expect("bounded worker accepts first request");
    let completion = (0..100).find_map(|_| {
        let completion = worker.drain_completions().into_iter().next();
        if completion.is_none() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        completion
    });
    let completion = completion.expect("worker returns bounded completion");
    assert_eq!(completion.job.generation, job.generation);
    assert!(
        completion
            .result
            .expect("missing cache is not a worker error")
            .is_none()
    );
}

#[test]
fn unavailable_worker_defers_without_publishing_completion() {
    let worker = CachedResidencyWorker::unavailable_for_test();
    let job = job(23);
    let root = tempfile::tempdir().expect("temporary cache root");
    let returned = worker
        .dispatch(root.path().to_path_buf(), job.clone())
        .expect_err("unavailable worker must defer demand");
    assert_eq!(returned, job);
    assert!(worker.drain_completions().is_empty());
}

#[test]
fn disconnected_worker_becomes_unavailable_instead_of_requeueing_forever() {
    let (request_tx, request_rx) = mpsc::sync_channel::<LoadRequest>(WORKER_QUEUE_CAPACITY);
    drop(request_rx);
    let worker = CachedResidencyWorker {
        requests: Some(request_tx),
        completions: None,
        thread: None,
        available: std::sync::atomic::AtomicBool::new(true),
    };
    let root = tempfile::tempdir().expect("temporary cache root");
    assert!(worker.dispatch(root.path().to_path_buf(), job(29)).is_err());
    assert!(!worker.is_available());
}
