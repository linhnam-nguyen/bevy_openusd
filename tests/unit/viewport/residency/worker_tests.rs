use super::*;
use bevy::asset::Assets;
use bevy::prelude::{App, Update};
use usd_model::HashDigest;
use usd_project::SceneId;

use crate::project::runtime::payload::{RuntimeAlphaMode, RuntimeMaterialTextures};
use crate::viewport::stage::SceneCachePresentation;
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
        version: crate::project::runtime::payload::RUNTIME_MATERIAL_VERSION,
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
        state: crate::project::cache::contract::SceneCacheState::Ready,
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
    let mut oversized = job(17);
    oversized.cpu_bytes = 16 * 1024 * 1024 + 1;
    let root = tempfile::tempdir().expect("temporary cache root");
    worker
        .dispatch(root.path().to_path_buf(), oversized.clone())
        .expect("bounded worker can run one oversized request alone");
    let waiting = job(18);
    assert_eq!(
        worker.dispatch(root.path().to_path_buf(), waiting.clone()),
        Err(waiting),
        "an oversized in-flight request holds the entire worker budget until publication"
    );
    let completion = (0..100).find_map(|_| {
        let completion = worker.drain_completions().into_iter().next();
        if completion.is_none() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        completion
    });
    let WorkerCompletion::Cached(completion) =
        completion.expect("worker returns bounded completion")
    else {
        panic!("cached request emitted a source completion");
    };
    assert_eq!(completion.job.generation, oversized.generation);
    assert!(
        completion
            .result
            .expect("missing cache is not a worker error")
            .is_none()
    );

    for generation in 18..22 {
        worker
            .dispatch(root.path().to_path_buf(), job(generation))
            .unwrap();
    }
    let mut generations = Vec::new();
    for _ in 0..100 {
        generations.extend(worker.drain_completions().into_iter().map(
            |completion| match completion {
                WorkerCompletion::Cached(completion) => completion.job.generation,
                WorkerCompletion::Source { .. } => panic!("unexpected source completion"),
            },
        ));
        if generations.len() == 4 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(generations, [18, 19, 20, 21]);
    exercise_canonical_source_preparation();
}

fn exercise_canonical_source_preparation() {
    use bevy::prelude::*;
    use usd_bevy::route::geom::{DeferHiddenSourceMeshes, DeferredSourceMesh};
    let fixture = usd_bevy::UsdSnippet::new(
        r#"#usda 1.0
def Mesh "Hidden" {
    token visibility = "invisible"
    point3f[] points = [(0,0,0), (1,0,0), (0,1,0)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0,1,2]
    float3[] extent = [(0,0,0), (1,1,0)]
}
def Mesh "HiddenWithoutExtent" {
    token visibility = "invisible"
    point3f[] points = [(0,0,0), (1,0,0), (0,1,0)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0,1,2]
}
def Mesh "LaterSmall" {
    token visibility = "invisible"
    point3f[] points = [(0,0,0), (1,0,0), (0,1,0)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0,1,2]
    float3[] extent = [(0,0,0), (1,1,0)]
}
def Mesh "LaterWide" {
    token visibility = "invisible"
    point3f[] points = [(0,0,0), (1,0,0), (1,1,0), (0,1,0)]
    int[] faceVertexCounts = [4]
    int[] faceVertexIndices = [0,1,2,3]
    float3[] extent = [(0,0,0), (1,1,0)]
}
"#,
    );
    let stage = fixture.open_stage().unwrap();
    let mut app = App::new();
    app.add_plugins((usd_bevy::UsdPlugin, usd_bevy::LiveStagePlugin))
        .init_resource::<DeferHiddenSourceMeshes>()
        .init_resource::<CachedResidencyWorker>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<bevy::pbr::StandardMaterial>>();
    app.world_mut()
        .insert_non_send(usd_bevy::LiveStage::new(stage));
    app.update();
    let world = app.world_mut();
    world.init_resource::<super::super::SourceDemandScan>();
    world.init_resource::<super::super::VisibilityChangeEpoch>();
    world.init_resource::<crate::viewport::scene::SceneAnchorIndex>();
    world.init_resource::<crate::viewport::scene::SelectedTargets>();
    let mut demand = Schedule::default();
    demand.add_systems(
        (
            super::super::advance_visibility_epoch,
            super::super::dispatch_source_meshes,
        )
            .chain(),
    );
    let entity = world
        .resource::<usd_bevy::PrimEntities>()
        .entity(world.resource::<usd_bevy::PathStore>(), "/Hidden")
        .unwrap();
    assert!(world.get::<DeferredSourceMesh>(entity).is_some());
    assert!(world.get::<Mesh3d>(entity).is_none());
    assert!(world.get::<usd_bevy::UsdLocalExtent>(entity).is_some());
    let no_extent = world
        .resource::<usd_bevy::PrimEntities>()
        .entity(
            world.resource::<usd_bevy::PathStore>(),
            "/HiddenWithoutExtent",
        )
        .unwrap();
    assert!(world.get::<DeferredSourceMesh>(no_extent).is_none());
    assert!(world.get::<Mesh3d>(no_extent).is_some());
    let first_identity = SourceMeshIdentity::capture(world, entity).unwrap();
    demand.run(world);
    assert!(
        world
            .resource::<super::super::SourceDemandScan>()
            .0
            .is_some()
    );
    let epoch = world.resource::<super::super::VisibilityChangeEpoch>().0;
    for _ in 0..100 {
        demand.run(world);
    }
    assert_eq!(
        world.resource::<super::super::VisibilityChangeEpoch>().0,
        epoch
    );
    assert!(
        world
            .get::<super::super::PreparingSourceMesh>(entity)
            .is_none()
    );
    world.entity_mut(entity).insert(Visibility::Visible);

    let publish_next = |world: &mut World| {
        let completion = (0..250)
            .find_map(|_| {
                let next = world
                    .resource::<CachedResidencyWorker>()
                    .drain_completions()
                    .into_iter()
                    .next();
                if next.is_none() {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                next
            })
            .expect("source worker responds");
        let WorkerCompletion::Source { identity, mesh, .. } = completion else {
            panic!("expected source geometry")
        };
        super::super::complete_source_mesh(world, identity, mesh);
    };

    demand.run(world);
    assert!(
        world
            .get::<super::super::PreparingSourceMesh>(entity)
            .is_some()
    );
    world.resource_mut::<usd_bevy::StageTime>().current = 2.0;
    publish_next(world);
    assert!(
        world.get::<Mesh3d>(entity).is_none(),
        "stale time may not attach the result"
    );
    super::super::dispatch_source_meshes(world);
    publish_next(world);
    let mesh = &world
        .get::<Mesh3d>(entity)
        .expect("current reveal attaches mesh")
        .0;
    assert_eq!(
        world
            .resource::<Assets<Mesh>>()
            .get(mesh)
            .unwrap()
            .count_vertices(),
        3
    );
    assert!(world.get::<DeferredSourceMesh>(entity).is_none());

    let prims = world.resource::<usd_bevy::PrimEntities>();
    let paths = world.resource::<usd_bevy::PathStore>();
    let small = prims.entity(paths, "/LaterSmall").unwrap();
    let wide = prims.entity(paths, "/LaterWide").unwrap();
    let inherited_parent = world.spawn(Visibility::Hidden).id();
    let override_parent = world.spawn(Visibility::Hidden).id();
    world
        .entity_mut(small)
        .insert((Visibility::Inherited, ChildOf(inherited_parent)));
    world
        .entity_mut(wide)
        .insert((Visibility::Inherited, ChildOf(override_parent)));
    demand.run(world);
    assert!(
        world
            .get::<super::super::PreparingSourceMesh>(small)
            .is_none()
    );
    assert!(
        world
            .get::<super::super::PreparingSourceMesh>(wide)
            .is_none()
    );
    *world.get_mut::<Visibility>(inherited_parent).unwrap() = Visibility::Visible;
    *world.get_mut::<Visibility>(wide).unwrap() = Visibility::Visible;
    demand.run(world);
    assert_eq!(
        world
            .resource::<CachedResidencyWorker>()
            .source_capacity()
            .unwrap()
            .jobs,
        WORKER_QUEUE_CAPACITY - 2
    );
    let mut order = Vec::new();
    for _ in 0..250 {
        for completion in world
            .resource::<CachedResidencyWorker>()
            .drain_completions()
        {
            let WorkerCompletion::Source { identity, mesh, .. } = completion else {
                panic!("expected source geometry")
            };
            order.push(identity.entity);
            super::super::complete_source_mesh(world, identity, mesh);
        }
        if order.len() == 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(order, [wide, small]);
    assert!(world.get::<Mesh3d>(small).is_some());
    assert!(world.get::<Mesh3d>(wide).is_some());
    demand.run(world);
    assert!(
        world
            .resource::<super::super::SourceDemandScan>()
            .0
            .is_some()
    );
    world.remove_non_send::<usd_bevy::LiveStage>();
    demand.run(world);
    assert!(
        world
            .resource::<super::super::SourceDemandScan>()
            .0
            .is_none()
    );
    world.insert_non_send(usd_bevy::LiveStage::new(fixture.open_stage().unwrap()));
    app.update();
    let world = app.world_mut();
    let replacement = world
        .resource::<usd_bevy::PrimEntities>()
        .entity(world.resource::<usd_bevy::PathStore>(), "/Hidden")
        .unwrap();
    assert_ne!(entity, replacement);
    super::super::publish_source_mesh(world, first_identity, None);
    assert!(world.get::<Mesh3d>(replacement).is_none());

    use crate::viewport::scene::{SceneAnchorIndex, SelectedTargets};
    let anchor = viewport_protocol::SceneAnchor::active_session("/Hidden");
    world.insert_resource(SceneAnchorIndex::from_test_entity(
        anchor.clone(),
        replacement,
    ));
    demand.run(world);
    world
        .resource_mut::<SelectedTargets>()
        .add(anchor.clone(), true)
        .unwrap();
    demand.run(world);
    world.resource_mut::<SelectedTargets>().clear().unwrap();
    publish_next(world);
    assert!(world.get::<Mesh3d>(replacement).is_none());
    world
        .resource_mut::<SelectedTargets>()
        .add(anchor, true)
        .unwrap();
    demand.run(world);
    publish_next(world);
    assert!(world.get::<Mesh3d>(replacement).is_some());
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
    let (request_tx, request_rx) = mpsc::sync_channel::<SequencedRequest>(WORKER_QUEUE_CAPACITY);
    drop(request_rx);
    let worker = CachedResidencyWorker {
        requests: Some(request_tx),
        completions: None,
        threads: Vec::new(),
        window: Mutex::default(),
        available: std::sync::atomic::AtomicBool::new(true),
    };
    let root = tempfile::tempdir().expect("temporary cache root");
    assert!(worker.dispatch(root.path().to_path_buf(), job(29)).is_err());
    assert!(!worker.is_available());
}
