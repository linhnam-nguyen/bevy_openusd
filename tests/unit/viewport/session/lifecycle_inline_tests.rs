use super::*;
use project_protocol::{ProjectActivationCommand, ProjectActivationReply, ProjectStageTarget};
use usd_project::{ProjectId, ProjectRoot};

fn fixture_path(file_name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/stages")
        .join(file_name)
}

fn activation_world() -> World {
    let mut world = World::new();
    world.insert_resource(PrimEntities::default());
    world.insert_resource(Spawned::default());
    world.insert_resource(StageInfo::default());
    world
}

fn assert_active_stage(world: &World, expected: &std::path::Path) {
    let expected_string = expected.to_string_lossy().into_owned();
    let expected_root = expected.parent().unwrap().to_path_buf();
    let expected_name = expected.file_name().unwrap().to_string_lossy();
    let requested = world.resource::<RequestedAsset>();
    assert_eq!(requested.root, expected_root);
    assert_eq!(requested.name, expected_name.as_ref());
    assert_eq!(world.resource::<StageHandle>().path, expected);
    assert_eq!(world.resource::<StageInfo>().path, expected_string);
    let expected_canonical =
        std::fs::canonicalize(expected).expect("canonicalize expected opened stage path");
    let layer_identifiers = world
        .get_non_send::<LiveStage>()
        .unwrap()
        .stage
        .layer_identifiers();
    assert!(
        layer_identifiers
            .iter()
            .filter_map(|identifier| {
                std::fs::canonicalize(std::path::Path::new(identifier)).ok()
            })
            .any(|identifier| identifier == expected_canonical),
        "opened layer identifiers {layer_identifiers:?} do not resolve to {expected_canonical:?}"
    );
}

#[test]
fn project_activation_replaces_a_with_b_and_failed_b_preserves_a() {
    let scene_a = fixture_path("hierarchy.usda");
    let scene_b = fixture_path("primitives.usda");
    let missing_b = fixture_path("missing-project-root.usda");
    let mut world = activation_world();

    activate_stage(&mut world, scene_a.clone()).unwrap();
    let session_a = world.get_non_send::<LiveStage>().unwrap().session_id();
    assert_active_stage(&world, &scene_a);

    activate_stage(&mut world, scene_b.clone()).unwrap();
    let session_b = world.get_non_send::<LiveStage>().unwrap().session_id();
    assert_ne!(session_a, session_b);
    assert_active_stage(&world, &scene_b);

    activate_stage(&mut world, scene_a.clone()).unwrap();
    let preserved_session = world.get_non_send::<LiveStage>().unwrap().session_id();
    let error = activate_stage(&mut world, missing_b.clone()).unwrap_err();

    assert_eq!(error, PROJECT_STAGE_OPEN_FAILURE);
    assert_eq!(
        world.get_non_send::<LiveStage>().unwrap().session_id(),
        preserved_session
    );
    assert_active_stage(&world, &scene_a);

    let command = ProjectActivationCommand::new(
        "activation-failed",
        2,
        ProjectId::new_v4(),
        ProjectStageTarget::ProjectRoot(ProjectRoot::Empty),
    );
    let encoded = serde_json::to_string(&ProjectActivationReply::failed(&command, error)).unwrap();
    assert!(!encoded.contains(&missing_b.to_string_lossy().to_string()));
    assert!(!encoded.contains("missing-project-root.usda"));
    assert!(encoded.contains(PROJECT_STAGE_OPEN_FAILURE));
}
