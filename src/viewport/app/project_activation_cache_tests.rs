use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

use project_protocol::{ProjectActivationCommand, ProjectStageTarget};
use tempfile::tempdir;
use usd_project::{
    ProjectId, ProjectManifestV1, ProjectRoot, SceneId, SceneManifestEntry, StorageKey,
};
use viewport_protocol::SessionId;

use super::*;
use crate::project::cache::{
    ProjectCacheDescriptor, ProjectCacheIdentity, ProjectCacheState, ProjectCacheStore,
    ProjectCacheTarget,
};
use crate::project::cache_hydration::default_project_cache_config_hash;
use crate::project::catalog::{
    manifest_store::ManifestStore, workspace_registry::WorkspaceRegistry,
};
use crate::project::scene::authoring::author_scene_atomic;

#[test]
fn activation_does_not_wait_for_inflight_cache_warm() {
    let directory = tempdir().expect("temporary activation workspace");
    let registry_path = directory.path().join("workspace.json");
    let project_root = directory.path().join("project");
    usd_git::Repository::init(&project_root).expect("initialize Project repository");
    let project_id = ProjectId::new_v4();
    let scene_id = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        project_id,
        "Immediate Cache Activation",
        ProjectRoot::Scene(scene_id),
        vec![SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("scene").expect("Scene storage key"),
            display_name: "Immediate Cache Activation".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(&project_root, &manifest).expect("write Project manifest");
    let scene_path = author_scene_atomic(&project_root, scene_id).expect("author Scene");
    let target = ProjectCacheTarget::Scene {
        id: scene_id.to_string(),
    };
    let identity = ProjectCacheIdentity::for_project(
        &project_root,
        target.clone(),
        viewport_protocol::RuntimeProfile::NativeMedium,
        default_project_cache_config_hash(),
    )
    .expect("compute cache identity");
    ProjectCacheStore::new(&project_root)
        .publish(
            &ProjectCacheDescriptor::new(identity.clone(), ProjectCacheState::Building, None)
                .expect("create Building descriptor"),
        )
        .expect("publish Building descriptor");

    let mut registry = WorkspaceRegistry::load(&registry_path).expect("load registry");
    registry
        .register(project_id, &project_root, None)
        .expect("register Project");
    let request = ProjectActivationRequest {
        session_id: SessionId::new("session-immediate-cache"),
        command: ProjectActivationCommand::new(
            "activation-immediate-cache",
            1,
            project_id,
            ProjectStageTarget::Scene(scene_id),
        ),
    };

    let started = Instant::now();
    let resolved = resolve_project_activation(Some(&registry_path), &request)
        .expect("activation preparation succeeds")
        .expect("Scene activation target");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "activation preparation waited for cache warm"
    );
    assert_eq!(
        resolved.path,
        fs::canonicalize(scene_path).expect("canonical Scene path")
    );
    assert_eq!(resolved.cache_identity.as_ref(), Some(&identity));
}

#[test]
fn real_project_hummingbird_activation_reaches_geometry_ready_and_playback() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/external/hummingbird.usdz");
    assert!(source.is_file(), "real Hummingbird fixture is present");

    let directory = tempdir().expect("temporary Hummingbird Project");
    let project_root = directory.path().join("project");
    usd_git::Repository::init(&project_root).expect("initialize Hummingbird Project");
    let project_id = ProjectId::new_v4();
    let scene_id = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        project_id,
        "Hummingbird Project",
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
    let scene_path = crate::project::scene::authoring::scene_path(&project_root, scene_id);
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
    crate::project::scene::adoption_authoring::author_scene_wrapper_to_path(
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

    let registry_path = directory.path().join("workspace.json");
    let mut registry = WorkspaceRegistry::load(&registry_path).expect("load test registry");
    registry
        .register(project_id, &project_root, None)
        .expect("register Hummingbird Project");
    let runtime = ProjectStageActivationRuntime::with_registry_path(Some(registry_path));
    let command = ProjectActivationCommand::new(
        "hummingbird-project-activation",
        1,
        project_id,
        ProjectStageTarget::Scene(scene_id),
    );
    let request = ProjectActivationRequest {
        session_id: SessionId::new("hummingbird-project-session"),
        command: command.clone(),
    };
    assert!(runtime.submit(request.clone()).is_none());
    let prepared = runtime
        .wait_for_prepared()
        .expect("Hummingbird activation preparation result");
    let target = prepared
        .target
        .expect("Hummingbird activation preparation succeeds")
        .expect("Hummingbird Scene target resolves");
    assert_eq!(
        target.path,
        fs::canonicalize(&scene_path).expect("canonical Scene wrapper")
    );
    assert_eq!(
        target.archive_paths,
        vec![fs::canonicalize(package_path).expect("canonical Hummingbird package")]
    );
    assert!(
        target.cache_identity.is_some(),
        "prepared cache identity is carried forward"
    );

    let mut production = ProductionActivationWorld::new();
    assert!(production.admit("hummingbird-project-session", &command));
    let reply = production.apply(
        "hummingbird-project-session",
        &command,
        Ok(Some(target.clone())),
    );
    assert!(matches!(
        reply.result,
        project_protocol::ProjectActivationResult::Activated { .. }
    ));

    let mut saw_first_geometry = false;
    let mut update_ticks = 0;
    for _ in 0..10_000 {
        update_ticks += 1;
        production.update();
        let state = production
            .world()
            .resource::<usd_bevy::ProgressiveProjectionState>();
        saw_first_geometry |= state.first_mesh_ms().is_some();
        if state.readiness() == usd_bevy::ProjectionReadiness::Ready {
            break;
        }
    }
    let world = production.world();
    let state = world.resource::<usd_bevy::ProgressiveProjectionState>();
    assert!(saw_first_geometry, "Hummingbird reached first geometry");
    assert!(
        update_ticks > 1,
        "bounded projection kept update ticks live"
    );
    assert_eq!(state.readiness(), usd_bevy::ProjectionReadiness::Ready);
    assert!(world.get_non_send::<usd_bevy::LiveStage>().is_some());
    assert!(!world.resource::<usd_bevy::AnimatedPrims>().0.is_empty());
    assert!(
        world
            .resource::<crate::viewport::animation::UsdStageTime>()
            .playing,
        "Hummingbird playback was initialized after projection readiness"
    );
    assert!(
        world
            .resource::<bevy::asset::Assets<bevy::image::Image>>()
            .iter()
            .next()
            .is_some()
    );
    let provenance = world.resource::<usd_bevy::route::material::MaterialProjectionProvenance>();
    let paths = world.resource::<usd_bevy::PathStore>();
    let prims = world.resource::<usd_bevy::PrimEntities>();
    assert!(prims.iter(paths).any(|(path, _)| {
        provenance.status(path)
            == Some(usd_bevy::route::material::MaterialProjectionStatus::AuthoredConversion)
    }));
    assert_eq!(
        world
            .resource::<usd_bevy::route::material::UsdTextureCache>()
            .stats()
            .archive_misses,
        0
    );
}
