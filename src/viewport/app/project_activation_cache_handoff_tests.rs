use std::{fs, path::Path};

use bevy::asset::{Assets, RenderAssetUsages};
use bevy::ecs::system::Commands;
use bevy::ecs::world::CommandQueue;
use bevy::mesh::{Mesh, PrimitiveTopology};
use project_protocol::{ProjectActivationCommand, ProjectStageTarget};
use tempfile::tempdir;
use usd_bevy::{LiveStage, PrimEntities};
use usd_model::{Bounds3, HashDigest};
use usd_project::{
    ProjectId, ProjectManifestV1, ProjectRoot, SceneId, SceneManifestEntry, SceneMemberId,
    StorageKey,
};

use super::ProductionActivationWorld;
use crate::project::cache::SceneCacheStore;
use crate::project::cache_contract::{
    SCENE_CACHE_INDEX_SCHEMA_VERSION, SCENE_SPATIAL_INDEX_SCHEMA_VERSION, SceneCacheAddress,
    SceneCacheDescriptorV3, SceneCacheIndex, SceneCacheOccurrence, SceneCacheState,
    SceneSpatialIndex,
};
use crate::project::cache_hydration::default_project_cache_config_hash;
use crate::project::catalog::manifest_store::ManifestStore;
use crate::project::scene::{adoption_authoring, authoring};
use crate::project::service::{ProjectStageActivationTarget, ProjectStagePresentationContext};
use crate::viewport::residency::{
    ScenePayloadKey, SceneResidencyOccurrence, SceneResidencyProjection, SceneSpatialPayload,
};
use crate::viewport::session::SceneCachePresentation;

#[test]
fn cache_first_handoff_uses_render_schedule_before_retiring_projection() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/external/hummingbird.usdz");
    let directory = tempdir().expect("temporary cache-first Hummingbird Project");
    let project_root = directory.path().join("project");
    usd_git::Repository::init(&project_root).expect("initialize Hummingbird Project");
    let project_id = ProjectId::new_v4();
    let scene_id = usd_project::SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        project_id,
        "Cache-first Hummingbird Project",
        ProjectRoot::Empty,
        vec![SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("hummingbird").expect("Hummingbird storage key"),
            display_name: "Hummingbird".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(&project_root, &manifest)
        .expect("write Hummingbird Project manifest");

    let scene_path = authoring::scene_path(&project_root, scene_id);
    let package_dir = project_root
        .join("imports/scenes")
        .join(scene_id.to_string());
    fs::create_dir_all(&package_dir).expect("create Hummingbird import directory");
    let package_path = package_dir.join("hummingbird.usdz");
    fs::copy(&source, &package_path).expect("copy Hummingbird package");
    let spatial = crate::project::spatial::inspect_source(&package_path)
        .expect("inspect Hummingbird source metadata");
    fs::create_dir_all(scene_path.parent().expect("Hummingbird scene directory"))
        .expect("create Hummingbird scene directory");
    adoption_authoring::author_scene_wrapper_to_path(
        &scene_path,
        &project_root,
        &scene_path,
        scene_id,
        &package_path,
        &package_path,
        &["/hummingbird_anim_hover_idle_long".to_owned()],
        "Hummingbird",
        &spatial,
        false,
    )
    .expect("write Hummingbird Scene wrapper");

    let config_hash = default_project_cache_config_hash();
    let mut descriptor = SceneCacheDescriptorV3::invalidated(scene_id, 1, config_hash);
    descriptor.state = SceneCacheState::Partial;
    let index = SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 1,
        entries: Vec::new(),
    };
    let spatial = SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 1,
        entries: Vec::new(),
    };
    let store = SceneCacheStore::new(&project_root);
    store
        .publish_generation(&descriptor, &index, &spatial)
        .expect("publish cache-first fixture");
    let scene_cache = store
        .load_activation(scene_id)
        .expect("load cache-first fixture")
        .expect("cache-first activation exists");

    let command = ProjectActivationCommand::new(
        "cache-first-hummingbird-animation",
        1,
        project_id,
        ProjectStageTarget::Scene(scene_id),
    );
    let target = ProjectStageActivationTarget {
        project_id,
        target: command.target.clone(),
        project_root: project_root.clone(),
        path: fs::canonicalize(&scene_path).expect("canonical Hummingbird Scene wrapper"),
        archive_paths: vec![fs::canonicalize(package_path).expect("canonical Hummingbird package")],
        cache_identity: None,
        scene_cache: Some(scene_cache),
        presentation: ProjectStagePresentationContext::default(),
    };

    let mut production = ProductionActivationWorld::new();
    assert!(production.admit("cache-first-hummingbird-session", &command));
    assert!(
        production
            .apply(
                "cache-first-hummingbird-session",
                &command,
                Ok(Some(target)),
            )
            .is_none()
    );
    assert_eq!(
        production
            .world()
            .resource::<SceneCachePresentation>()
            .generation,
        1
    );
    assert!(
        production
            .world()
            .get_resource::<super::super::PendingCanonicalStageActivation>()
            .is_some()
    );
    assert!(production.world().get_non_send::<LiveStage>().is_none());
    assert_ne!(
        production
            .world()
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness(),
        usd_bevy::ProjectionReadiness::Ready
    );
    install_visible_cache_occurrence(&mut production, scene_id);

    production.update();
    assert!(production.world().get_non_send::<LiveStage>().is_none());
    production.mark_cache_rendered_for_test();
    production.update();
    for _ in 0..10_000 {
        if production
            .world()
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness()
            == usd_bevy::ProjectionReadiness::Ready
        {
            break;
        }
        production.update();
    }

    let (live_identity_before, prim_count_before) = {
        let world = production.world_mut();
        assert_eq!(
            world
                .resource::<usd_bevy::ProgressiveProjectionState>()
                .readiness(),
            usd_bevy::ProjectionReadiness::Ready
        );
        let pending = world.resource::<crate::viewport::session::PendingCanonicalVisualHandoff>();
        assert!(pending.canonical_ready);
        assert!(!pending.canonical_frame_rendered);
        assert!(world.get_resource::<SceneCachePresentation>().is_some());
        assert!(
            world
                .resource::<SceneResidencyProjection>()
                .active_entity_count_for_test()
                > 0
        );
        assert_eq!(
            world
                .query::<&SceneResidencyOccurrence>()
                .iter(world)
                .count(),
            1
        );
        let live = world
            .get_non_send::<LiveStage>()
            .expect("canonical LiveStage");
        assert_eq!(
            live.stage.root_layer().identifier(),
            fs::canonicalize(&scene_path)
                .expect("canonical Scene wrapper")
                .to_string_lossy()
        );
        assert!(!world.resource::<usd_bevy::AnimatedPrims>().0.is_empty());
        assert!(
            world
                .resource::<crate::viewport::animation::UsdStageTime>()
                .playing
        );
        assert!(
            world
                .resource::<super::super::ProjectActivationAuthorityRuntime>()
                .0
                .active()
                .is_some()
        );
        (
            live.stage_identity(),
            world.resource::<PrimEntities>().len(),
        )
    };

    production.update();
    {
        let world = production.world();
        assert!(world.get_resource::<SceneCachePresentation>().is_some());
        assert!(
            world
                .resource::<crate::viewport::session::PendingCanonicalVisualHandoff>()
                .canonical_frame_rendered
        );
    }

    production.update();
    let world = production.world_mut();
    let live = world
        .get_non_send::<LiveStage>()
        .expect("canonical LiveStage");
    assert_eq!(live.stage_identity(), live_identity_before);
    assert_eq!(world.resource::<PrimEntities>().len(), prim_count_before);
    assert!(world.resource::<PrimEntities>().len() > 0);
    assert_eq!(
        world
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness(),
        usd_bevy::ProjectionReadiness::Ready
    );
    assert_eq!(
        world
            .resource::<SceneResidencyProjection>()
            .active_entity_count_for_test(),
        0
    );
    assert_eq!(
        world
            .query::<&SceneResidencyOccurrence>()
            .iter(world)
            .count(),
        0
    );
    assert!(world.get_resource::<SceneCachePresentation>().is_none());
    assert!(
        world
            .get_resource::<crate::viewport::session::PendingCanonicalVisualHandoff>()
            .is_none()
    );
    assert!(!world.resource::<usd_bevy::AnimatedPrims>().0.is_empty());
    assert!(
        world
            .resource::<crate::viewport::animation::UsdStageTime>()
            .playing
    );
}

fn install_visible_cache_occurrence(production: &mut ProductionActivationWorld, scene_id: SceneId) {
    let payload = SceneSpatialPayload {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::Member(SceneMemberId::new_v4()),
        },
        payload_key: ScenePayloadKey {
            scene_id,
            blob_hash: HashDigest::new([9; HashDigest::BYTE_LEN]),
        },
        transform: usd_project::ScenePlacementTransform::IDENTITY,
        bounds: Bounds3 {
            min: [-1.0; 3],
            max: [1.0; 3],
        },
        cpu_bytes: 8,
        gpu_bytes: 8,
    };
    let world = production.world_mut();
    let handle = world.resource_mut::<Assets<Mesh>>().add(Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    ));
    let mut projection = world
        .remove_resource::<SceneResidencyProjection>()
        .expect("cache projection resource");
    projection.install_scene(std::slice::from_ref(&payload));
    let mut queue = CommandQueue::default();
    {
        let mut commands = Commands::new(&mut queue, &*world);
        projection.attach_payload(payload.payload_key, handle, &mut commands);
    }
    queue.apply(world);
    world.insert_resource(projection);
}
