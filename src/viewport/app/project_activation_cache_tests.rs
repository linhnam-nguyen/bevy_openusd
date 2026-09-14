use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

use bevy::prelude::Transform;
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
    assert!(
        resolved.cache_identity.is_none(),
        "Scene activation does not compute the legacy full Project identity"
    );
    assert!(resolved.scene_cache.is_none());
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
    assert!(target.cache_identity.is_none());
    assert!(target.scene_cache.is_none());

    let mut production = ProductionActivationWorld::new();
    assert!(production.admit("hummingbird-project-session", &command));
    let reply = production.apply(
        "hummingbird-project-session",
        &command,
        Ok(Some(target.clone())),
    );
    assert!(matches!(
        reply.expect("activation completion reply").result,
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
    let world = production.world_mut();
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
    let joint_count = world
        .query::<&usd_bevy::route::skel::UsdJoint>()
        .iter(world)
        .count();
    let driver_count = world
        .query::<&usd_bevy::route::skel::UsdSkelAnimDriver>()
        .iter(world)
        .count();
    assert!(
        joint_count > 0 && driver_count > 0,
        "Project wrapper must project skeletal animation bindings (joints={joint_count}, drivers={driver_count}, animated={:?})",
        world.resource::<usd_bevy::AnimatedPrims>().0
    );

    let (start, end) = {
        let live = world
            .get_non_send::<usd_bevy::LiveStage>()
            .expect("canonical Project LiveStage");
        (live.stage.start_time_code(), live.stage.end_time_code())
    };
    let t0 = start + (end - start) * 0.25;
    let t1 = start + (end - start) * 0.75;
    let transform_t0 = seek_project_time(&mut production, t0);
    let transform_t1 = seek_project_time(&mut production, t1);
    let transform_round_trip = seek_project_time(&mut production, t0);
    assert_ne!(
        transform_t0, transform_t1,
        "Project Scene wrapper must preserve visible transform animation"
    );
    assert_eq!(
        transform_t0, transform_round_trip,
        "Project animation must round-trip deterministically"
    );
}

fn seek_project_time(production: &mut ProductionActivationWorld, time_code: f64) -> u64 {
    {
        let mut clock = production
            .world_mut()
            .resource_mut::<crate::viewport::animation::UsdStageTime>();
        clock.playing = false;
        clock.seconds = (time_code - clock.start_time_code) / clock.time_codes_per_second;
    }
    production.update();
    production.update();
    let world = production.world_mut();
    let mut query = world.query::<(&usd_bevy::prim_ref::UsdPrimRef, &Transform)>();
    let mut transforms = query
        .iter(world)
        .map(|(prim, transform)| (prim.path.clone(), *transform))
        .collect::<Vec<_>>();
    let mut joints = world.query::<(&usd_bevy::route::skel::UsdJoint, &Transform)>();
    transforms.extend(
        joints
            .iter(world)
            .map(|(joint, transform)| (format!("joint:{}", joint.path), *transform)),
    );
    transforms.sort_by(|(left, _), (right, _)| left.cmp(right));
    project_transform_signature(&transforms)
}

fn project_transform_signature(samples: &[(String, Transform)]) -> u64 {
    const FNV_OFFSET: u64 = 14_695_981_039_346_656_037;
    const FNV_PRIME: u64 = 1_099_511_628_211;
    let mut hash = FNV_OFFSET;
    for (path, transform) in samples {
        for byte in (path.len() as u64)
            .to_le_bytes()
            .iter()
            .chain(path.as_bytes())
        {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME);
        }
        for value in transform
            .translation
            .to_array()
            .into_iter()
            .chain(transform.rotation.to_array())
            .chain(transform.scale.to_array())
        {
            let quantized = (f64::from(value) * 1_000_000.0).round() as i64;
            for byte in quantized.to_le_bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME);
            }
        }
    }
    hash
}

#[cfg(test)]
#[path = "project_activation_skel_tests.rs"]
mod skel_tests;
