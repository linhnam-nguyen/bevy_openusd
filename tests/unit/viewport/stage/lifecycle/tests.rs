use super::*;
use project_protocol::{ProjectActivationCommand, ProjectActivationReply, ProjectStageTarget};
use usd_project::{ProjectId, ProjectRoot};

fn fixture_path(file_name: &str) -> std::path::PathBuf {
    usd_support::resolve_fixture_path_unchecked(file_name)
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

    // The standalone loader shares the same last-good ownership boundary.
    open::open_stage(&mut world, missing_b.clone());
    assert_eq!(
        world.get_non_send::<LiveStage>().map(LiveStage::session_id),
        Some(preserved_session)
    );
    assert_active_stage(&world, &scene_a);

    // Reloading the active file must not erase a canonical edit held in memory.
    let live = world.get_non_send::<LiveStage>().unwrap();
    usd_bevy::authoring::define_prim(&live.stage, "/UnsavedReloadEdit", "Xform").unwrap();
    let before_reload = live.authoring_generation();
    world.insert_resource(ReloadRequest { requested: true });
    handle_usd_hot_reload(&mut world);
    let live = world.get_non_send::<LiveStage>().unwrap();
    assert_eq!(live.session_id(), preserved_session);
    assert_eq!(live.authoring_generation(), before_reload);
    assert!(usd_bevy::authoring::prim_exists(
        &live.stage,
        "/UnsavedReloadEdit"
    ));

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

    verify_visual_unload_retains_authoritative_payload(&mut world);

    let package =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/external/usdz_sample.usdz");
    let package_bytes = std::fs::read(&package).unwrap();
    open::open_stage(&mut world, package.clone());
    assert_eq!(world.resource::<StageHandle>().path, package);
    assert!(
        world
            .get_non_send::<LiveStage>()
            .unwrap()
            .stage
            .default_prim()
            .is_some()
    );
    assert_eq!(std::fs::read(&package).unwrap(), package_bytes);
}

fn verify_visual_unload_retains_authoritative_payload(world: &mut World) {
    let source = tempfile::tempdir().unwrap();
    let root = source.path().join("root.usda");
    let source_bytes = br#"#usda 1.0
def Xform "Model" (
    prepend payload = @./payload.usda@</PayloadRoot>
)
{
}
"#;
    std::fs::write(&root, source_bytes).unwrap();
    std::fs::write(
        source.path().join("payload.usda"),
        "#usda 1.0\ndef Xform \"PayloadRoot\" { def Xform \"Child\" {} }\n",
    )
    .unwrap();
    open::open_stage(world, root.clone());
    assert_active_stage(world, &root);
    let live = world.get_non_send::<LiveStage>().unwrap();
    assert!(usd_bevy::authoring::prim_exists(
        &live.stage,
        "/Model/Child"
    ));
    let generation = live.authoring_generation();
    let initial_visual = live.visual_context_id();
    live.unload_payload("/Model").unwrap();
    assert!(usd_bevy::authoring::prim_exists(
        &live.stage,
        "/Model/Child"
    ));
    assert_eq!(live.authoring_generation(), generation);
    assert!(
        live.drain_change_batch().is_none(),
        "visual payload changes must not advance canonical publication"
    );
    usd_bevy::authoring::define_prim(&live.stage, "/Model/Child/Added", "Xform").unwrap();
    live.load_payload("/Model").unwrap();
    assert_ne!(
        live.visual_context_id(),
        initial_visual,
        "a published authored revision invalidates cached picks"
    );
    assert!(usd_bevy::authoring::prim_exists(
        &live.stage,
        "/Model/Child/Added"
    ));
    assert_eq!(std::fs::read(&root).unwrap(), source_bytes);
    let last_good_session = live.session_id();
    let broken = source.path().join("broken.usda");
    std::fs::write(
        &broken,
        "#usda 1.0\ndef Xform \"Broken\" (prepend references = @./missing.usda@</Missing>) {}\n",
    )
    .unwrap();
    open::open_stage(world, broken);
    assert_eq!(
        world.get_non_send::<LiveStage>().unwrap().session_id(),
        last_good_session
    );
    assert_active_stage(world, &root);
}
