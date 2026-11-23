use bevy::prelude::*;

use crate::live::{LiveStage, LiveStagePlugin, PathStore, PrimEntities};
use crate::snippet::UsdSnippet;

#[test]
fn test_live_stage_load_and_unload_payload_api_preserves_projection() {
    let usda = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/live/payload_1.usda"
    ));
    let stage = UsdSnippet::new(usda)
        .open_stage()
        .expect("payload stage opens");
    let live = LiveStage::new(stage);
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(live);
    app.update();

    let prim_entities = app.world().resource::<PrimEntities>();
    assert!(
        prim_entities
            .entity(app.world().resource::<PathStore>(), "/World/PayloadPrim")
            .is_some()
    );

    let live = app.world().get_non_send::<LiveStage>().unwrap();
    live.unload_payload("/World/PayloadPrim")
        .expect("unload payload succeeds");
    live.load_payload("/World/PayloadPrim")
        .expect("load payload succeeds");
}

#[test]
fn projects_skel_root_skeleton_and_skinning_data() {
    use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
    use openusd::usd::{PrimPredicate, SchemaBase};
    use openusd_schemas::skel::{
        BindingAPI as SkelBindingAPI, Cache, Root, Skeleton, SkeletonSchema,
    };

    let stage = usd_support::open_fixture_stage("skel.usda");
    let mut app = App::new();
    app.add_plugins(crate::UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .add_plugins(bevy::transform::TransformPlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<SkinnedMeshInverseBindposes>>();
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();

    let world = app.world_mut();
    let drivers: Vec<_> = world
        .query::<&crate::route::skel::UsdSkelAnimDriver>()
        .iter(world)
        .cloned()
        .collect();
    assert_eq!(drivers.len(), 1);
    assert_eq!(
        drivers[0].skeleton_joints,
        vec!["root".to_string(), "root/tip".to_string()]
    );
    assert_eq!(drivers[0].animation_source_path, "/World/Rig/Anim");

    let mut joint_paths: Vec<_> = world
        .query::<&crate::route::skel::UsdJoint>()
        .iter(world)
        .map(|j| (j.skeleton_path.clone(), j.path.clone()))
        .collect();
    joint_paths.sort();
    assert_eq!(
        joint_paths,
        vec![
            ("/World/Rig/Skel".to_string(), "root".to_string()),
            ("/World/Rig/Skel".to_string(), "root/tip".to_string()),
        ]
    );

    let live = world.get_non_send::<LiveStage>().expect("live stage");
    let root_path = openusd::sdf::path("/World/Rig").unwrap();
    let root = Root::get(&live.stage, root_path)
        .unwrap()
        .expect("SkelRoot");
    let mut cache = Cache::new();
    cache.populate(&root, PrimPredicate::DEFAULT).unwrap();
    let bindings = cache
        .skel_bindings(&root, PrimPredicate::DEFAULT)
        .expect("bindings discover");
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].skeleton().path().as_str(), "/World/Rig/Skel");
    assert_eq!(
        bindings[0]
            .skeleton()
            .animation_source()
            .unwrap()
            .as_ref()
            .map(|a| a.path().as_str()),
        Some("/World/Rig/Anim")
    );
    let arm_path = openusd::sdf::path("/World/Rig/Arm").unwrap();
    let skel_path = openusd::sdf::path("/World/Rig/Skel").unwrap();
    let skel = Skeleton::get(&live.stage, skel_path)
        .unwrap()
        .expect("skeleton exists");
    assert_eq!(
        skel.joints_attr()
            .cast::<Vec<String>>()
            .unwrap()
            .unwrap_or_default(),
        vec!["root".to_string(), "root/tip".to_string()]
    );
    let binding_api = SkelBindingAPI::get(&live.stage, arm_path)
        .unwrap()
        .expect("Arm has SkelBindingAPI");
    let indices = match binding_api
        .joint_indices_attr()
        .get::<openusd::vt::Value>()
        .unwrap()
    {
        Some(openusd::vt::Value::IntVec(v)) => v,
        other => panic!("expected IntVec jointIndices, got {other:?}"),
    };
    let weights = match binding_api
        .joint_weights_attr()
        .get::<openusd::vt::Value>()
        .unwrap()
    {
        Some(openusd::vt::Value::FloatVec(v)) => v,
        other => panic!("expected FloatVec jointWeights, got {other:?}"),
    };
    assert_eq!(indices.len(), 6);
    assert_eq!(weights.len(), 6);
    assert_eq!(indices[0], 0);
    assert!((weights[0] - 1.0).abs() < 1e-5);
    assert!((weights[2] - 0.5).abs() < 1e-5);
    assert!((weights[3] - 0.5).abs() < 1e-5);
}

#[test]
fn visual_payload_batch_preserves_canonical_nested_geometry() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("root.usda");
    let parent = dir.path().join("parent.usda");
    let child = dir.path().join("child.usda");
    let root_bytes = br#"#usda 1.0
def Xform "Parent" (payload = @./parent.usda@</Parent>) {}
def Xform "Sibling" {}
"#;
    std::fs::write(&root, root_bytes)?;
    std::fs::write(
        &parent,
        br#"#usda 1.0
def Xform "Parent" {
    def Xform "Child" (payload = @./child.usda@</Child>) {}
}
"#,
    )?;
    std::fs::write(
        &child,
        br#"#usda 1.0
def Xform "Child" { def Cube "Geometry" {} }
"#,
    )?;
    let stage = LiveStage::open_document(&root)?;
    let canonical = stage.stage.load_set()?;
    let parent_path = "/Parent".to_owned();
    let child_path = "/Parent/Child".to_owned();
    let geometry = "/Parent/Child/Geometry";
    assert_eq!(
        stage.visual_loadable_paths()?,
        [parent_path.clone(), child_path.clone()]
    );
    stage.apply_visual_load_batch(&[], &[child_path.clone(), parent_path.clone()])?;
    assert!(stage.visual_loaded_paths()?.is_empty());
    assert!(stage.stage.prim(geometry)?.is_defined()?);
    assert_eq!(stage.stage.load_set()?, canonical);
    assert!(stage.drain_change_batch().is_none());
    stage.apply_visual_load_batch(&[parent_path, child_path], &[])?;
    assert_eq!(stage.visual_loaded_paths()?.len(), 2);
    assert_eq!(std::fs::read(&root)?, root_bytes);

    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut()
        .insert_non_send(LiveStage::open_document(&root)?);
    app.update();
    let sibling = app
        .world()
        .resource::<PrimEntities>()
        .entity(app.world().resource::<PathStore>(), "/Sibling")
        .unwrap();
    let generation = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>()
        .generation();
    let child = "/Parent/Child".to_owned();
    app.world()
        .get_non_send::<LiveStage>()
        .unwrap()
        .apply_visual_load_batch(&[], std::slice::from_ref(&child))?;
    for _ in 0..3 {
        app.update();
    }
    assert!(
        app.world()
            .resource::<PrimEntities>()
            .entity(app.world().resource::<PathStore>(), geometry)
            .is_none()
    );
    app.world()
        .get_non_send::<LiveStage>()
        .unwrap()
        .apply_visual_load_batch(&[child], &[])?;
    for _ in 0..3 {
        app.update();
    }
    let world = app.world();
    assert_eq!(
        world
            .resource::<crate::live::ProgressiveProjectionState>()
            .generation(),
        generation
    );
    assert_eq!(
        world
            .resource::<PrimEntities>()
            .entity(world.resource::<PathStore>(), "/Sibling"),
        Some(sibling)
    );
    assert!(
        world
            .resource::<PrimEntities>()
            .entity(world.resource::<PathStore>(), geometry)
            .is_some()
    );
    Ok(())
}

#[test]
fn authored_lod_uses_only_the_visual_session_and_rejects_changed_identity() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("lod.usda");
    let source = br#"#usda 1.0
def Xform "Model" (
    variants = { string lod = "LOD0" string look = "Blue" }
    prepend variantSets = ["lod", "look"]
) {
    variantSet "lod" = {
        "LOD0" { def Cube "Part" {} }
        "LOD1" { def Cube "Part" {} }
        "LOD2" { def Cube "OtherName" {} }
    }
    variantSet "look" = {
        "Blue" { string userProperties:theme = "blue" }
        "Red" { string userProperties:theme = "red" }
    }
}
"#;
    std::fs::write(&root, source)?;
    let live = LiveStage::open_document(&root)?;
    let path = "/Model";
    assert_eq!(
        live.authored_lod_options(path)?,
        [(
            "lod".into(),
            vec!["LOD0".into(), "LOD1".into(), "LOD2".into()]
        )]
    );
    assert!(live.select_visual_authored_lod(path, "lod", Some("LOD1"))?);
    assert!(!live.visual_lod_matches_canonical(path, "lod")?);
    assert!(
        live.select_visual_authored_lod(path, "lod", Some("LOD2"))
            .is_err()
    );
    assert!(
        live.select_visual_authored_lod(path, "look", Some("Red"))
            .is_err()
    );
    {
        let visual = live.projection_stage();
        let display = visual.prim(path)?;
        assert_eq!(
            display.variant_set("lod").selection()?.as_deref(),
            Some("LOD1")
        );
        assert_eq!(
            display.variant_set("look").selection()?.as_deref(),
            Some("Blue")
        );
        assert_eq!(
            live.stage
                .prim(path)?
                .variant_set("lod")
                .selection()?
                .as_deref(),
            Some("LOD0")
        );
    }
    assert!(live.select_visual_authored_lod(path, "lod", None)?);
    assert!(live.visual_lod_matches_canonical(path, "lod")?);
    assert_eq!(
        live.projection_stage()
            .prim(path)?
            .variant_set("lod")
            .selection()?
            .as_deref(),
        Some("LOD0")
    );
    assert!(live.drain_change_batch().is_none());
    assert_eq!(std::fs::read(root)?, source);
    Ok(())
}

#[test]
fn authored_lod_clear_removes_opinion_while_payload_is_unloaded() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("main.usda");
    let payload = dir.path().join("payload.usda");
    std::fs::write(
        &root,
        br#"#usda 1.0
def Xform "Owner" (payload = @./payload.usda@</Root>) {}
"#,
    )?;
    let source = br#"#usda 1.0
def Xform "Root" {
    def Xform "Model" (variants = { string lod = "LOD0" } prepend variantSets = "lod") {
        variantSet "lod" = {
            "LOD0" { def Cube "Part" {} }
            "LOD1" { def Cube "Part" {} }
        }
    }
}
"#;
    std::fs::write(&payload, source)?;
    let live = LiveStage::open_document(&root)?;
    let model = "/Owner/Model";
    assert!(live.select_visual_authored_lod(model, "lod", Some("LOD1"))?);
    assert!(!live.visual_lod_matches_canonical(model, "lod")?);
    live.apply_visual_load_batch(&[], &["/Owner".to_owned()])?;
    assert!(live.visual_loaded_paths()?.is_empty());
    assert!(live.select_visual_authored_lod(model, "lod", None)?);
    assert!(live.visual_lod_matches_canonical(model, "lod")?);
    assert!(!live.select_visual_authored_lod(model, "lod", None)?);
    live.apply_visual_load_batch(&["/Owner".to_owned()], &[])?;
    assert_eq!(
        live.projection_stage()
            .prim(model)?
            .variant_set("lod")
            .selection()?
            .as_deref(),
        Some("LOD0"),
    );
    assert_eq!(std::fs::read(payload)?, source);
    Ok(())
}
