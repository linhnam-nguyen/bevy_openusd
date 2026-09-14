use std::{fs, path::Path};

use project_protocol::{ProjectActivationCommand, ProjectStageTarget};
use tempfile::tempdir;
use usd_project::{ProjectId, ProjectManifestV1, ProjectRoot, SceneCompositionGraph};
use viewport_protocol::SessionId;
use viewport_streaming::ProjectActivationRequest;

use crate::project::catalog::{
    manifest_store::ManifestStore, workspace_registry::WorkspaceRegistry,
};
use crate::project::scene::inspection::inspect_composition;
use crate::project::scene::{adoption::SceneAdoptionRequest, adoption::adopt_scene_atomic};

use super::super::ProductionActivationWorld;

#[test]
fn project_wrapper_preserves_external_skel_animation_dependency() {
    let source_fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/skel_test_composed.usda");
    let directory = tempdir().expect("temporary skeletal Project");
    let source = directory.path().join("external-skel.usda");
    fs::copy(&source_fixture, &source).expect("copy skeletal source fixture");
    let project_root = directory.path().join("project");
    usd_git::Repository::init(&project_root).expect("initialize skeletal Project");

    let project_id = ProjectId::new_v4();
    let base_manifest = ProjectManifestV1::new(
        project_id,
        "External Skel Animation Project",
        ProjectRoot::Empty,
        Vec::new(),
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(&project_root, &base_manifest)
        .expect("write skeletal Project manifest");
    let inspection = inspect_composition(&source).expect("inspect skeletal source");
    assert_eq!(inspection.root_prims, vec!["/Animations", "/Character"]);

    let adopted = adopt_scene_atomic(SceneAdoptionRequest {
        project_root: &project_root,
        source: &source,
        inspection: &inspection,
        name: "External Skel Animation",
        base_manifest: &base_manifest,
        graph: &SceneCompositionGraph::default(),
        parent_scene_id: None,
        parent_members: &[],
        target_scene_id: None,
        set_as_root: true,
        placement: usd_project::ScenePlacementTransform::IDENTITY,
        linked_source: None,
    })
    .expect("adopt skeletal source through the real Scene wrapper path");
    let scene_id = adopted.scene_id;

    let registry_path = directory.path().join("workspace.json");
    let mut registry = WorkspaceRegistry::load(&registry_path).expect("load Project registry");
    registry
        .register(project_id, &project_root, None)
        .expect("register skeletal Project");
    let runtime =
        super::super::ProjectStageActivationRuntime::with_registry_path(Some(registry_path));
    let command = ProjectActivationCommand::new(
        "external-skel-project-activation",
        1,
        project_id,
        ProjectStageTarget::Scene(scene_id),
    );
    let request = ProjectActivationRequest {
        session_id: SessionId::new("external-skel-project-session"),
        command: command.clone(),
    };
    assert!(runtime.submit(request).is_none());
    let prepared = runtime
        .wait_for_prepared()
        .expect("skeletal activation preparation result");
    let target = prepared
        .target
        .expect("skeletal activation preparation succeeds")
        .expect("skeletal Scene target resolves");
    assert_eq!(
        target.path,
        fs::canonicalize(&adopted.scene_path).expect("canonical skeletal Scene wrapper")
    );

    let mut production = ProductionActivationWorld::new();
    assert!(production.admit("external-skel-project-session", &command));
    let reply = production.apply("external-skel-project-session", &command, Ok(Some(target)));
    assert!(matches!(
        reply.expect("skeletal activation completion reply").result,
        project_protocol::ProjectActivationResult::Activated { .. }
    ));

    for _ in 0..10_000 {
        production.update();
        if production
            .world()
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness()
            == usd_bevy::ProjectionReadiness::Ready
        {
            break;
        }
    }
    let world = production.world_mut();
    assert_eq!(
        world
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness(),
        usd_bevy::ProjectionReadiness::Ready
    );
    let (start, end, root_layer_identifier) = {
        let live = world
            .get_non_send::<usd_bevy::LiveStage>()
            .expect("canonical skeletal Project LiveStage");
        (
            live.stage.start_time_code(),
            live.stage.end_time_code(),
            live.stage.root_layer().identifier().to_owned(),
        )
    };
    assert_eq!(
        root_layer_identifier,
        fs::canonicalize(&adopted.scene_path)
            .expect("canonical skeletal Scene wrapper")
            .to_string_lossy()
    );
    let driver_sources = {
        let mut query = world.query::<&usd_bevy::route::skel::UsdSkelAnimDriver>();
        query
            .iter(world)
            .map(|driver| driver.animation_source_path.clone())
            .collect::<Vec<_>>()
    };
    assert!(
        driver_sources.iter().any(|path| path.ends_with("/Walk")),
        "Project wrapper must preserve the external /Animations/Walk relationship: {driver_sources:?}"
    );
    assert!(
        world
            .query::<&usd_bevy::route::skel::UsdJoint>()
            .iter(world)
            .count()
            >= 2,
        "Project wrapper must project the complete skeleton joint hierarchy"
    );

    let t0 = start + (end - start) * 0.25;
    let t1 = start + (end - start) * 0.75;
    let at_t0 = production.seek_animation_signature(t0);
    let at_t1 = production.seek_animation_signature(t1);
    let round_trip = production.seek_animation_signature(t0);
    assert_ne!(
        at_t0, at_t1,
        "Project skeletal wrapper must preserve joint motion"
    );
    assert_eq!(
        at_t0, round_trip,
        "Project skeletal wrapper animation must round-trip deterministically"
    );
}
