use bevy::prelude::*;
use openusd::usd::Stage;

use crate::live::{AnimatedPrims, LiveStage, LiveStagePlugin, PerformanceCounters};
use crate::prim_ref::UsdPrimRef;
use crate::{StageTime, UsdPlugin};

fn animated_app() -> App {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/anim_test_simple.usda");
    let stage =
        Stage::open(path.to_str().expect("fixture path is valid")).expect("animated fixture opens");
    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .add_plugins(bevy::transform::TransformPlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();
    app
}

#[test]
fn stage_time_uses_prebound_animation_without_structural_work() {
    let mut app = animated_app();
    let cube = {
        let world = app.world_mut();
        let mut query = world.query::<(Entity, &UsdPrimRef)>();
        query
            .iter(world)
            .find(|(_, prim)| prim.path == "/World/Cube")
            .map(|(entity, _)| entity)
            .expect("animated cube exists")
    };
    let before = app
        .world()
        .get::<Transform>(cube)
        .expect("cube transform before sampling")
        .rotation;
    {
        let mut counters = app.world_mut().resource_mut::<PerformanceCounters>();
        counters.enabled = true;
        counters.reset();
    }
    let rebuilds = app
        .world()
        .resource::<PerformanceCounters>()
        .animation_runtime_rebuilds;

    app.world_mut().resource_mut::<StageTime>().current = 30.0;
    app.update();

    let counters = app.world().resource::<PerformanceCounters>();
    assert_eq!(counters.stage_time_changes, 1);
    assert_eq!(counters.animation_runtime_samples, 1);
    assert_eq!(counters.animation_runtime_rebuilds, rebuilds);
    assert_eq!(counters.animation_generic_patch_calls, 0);
    assert_eq!(counters.animation_usd_path_parses, 0);
    assert_eq!(counters.animation_read_mesh_calls, 0);
    assert_eq!(counters.animation_mesh_allocations, 0);
    assert_eq!(counters.animation_material_allocations, 0);

    let after = app
        .world()
        .get::<Transform>(cube)
        .expect("cube transform after sampling")
        .rotation;
    assert_ne!(
        before, after,
        "pre-bound animation still updates transforms"
    );
}

#[test]
fn replacement_stage_with_same_start_time_receives_initial_animation_sample() {
    let stage_a = crate::snippet::UsdSnippet::new(
        r#"#usda 1.0
(
    startTimeCode = 0
    endTimeCode = 1
)
def Xform "World"
{
    def Xform "Animated"
    {
        float xformOp:rotateY.timeSamples = {
            0: 0,
            1: 90,
        }
        uniform token[] xformOpOrder = ["xformOp:rotateY"]
    }
}
"#,
    )
    .open_stage()
    .expect("stage A opens");
    let stage_b = crate::snippet::UsdSnippet::new(
        r#"#usda 1.0
(
    startTimeCode = 0
    endTimeCode = 1
)
def Xform "World"
{
    def Xform "Animated"
    {
        float xformOp:rotateY.timeSamples = {
            0: 180,
            1: 270,
        }
        uniform token[] xformOpOrder = ["xformOp:rotateY"]
    }
}
"#,
    )
    .open_stage()
    .expect("stage B opens");

    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .add_plugins(bevy::transform::TransformPlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut().insert_non_send(LiveStage::new(stage_a));
    app.update();
    app.world_mut()
        .resource_mut::<PerformanceCounters>()
        .enabled = true;
    app.world_mut()
        .resource_mut::<PerformanceCounters>()
        .reset();

    let replacement_identity = {
        let mut live = app
            .world_mut()
            .get_non_send_mut::<LiveStage>()
            .expect("live stage");
        live.replace_stage(stage_b);
        live.stage_identity()
    };
    for _ in 0..32 {
        app.update();
        let state = app
            .world()
            .resource::<crate::live::ProgressiveProjectionState>();
        if state.readiness() == crate::live::ProjectionReadiness::Ready
            && state.session_id() == Some(replacement_identity.0)
        {
            break;
        }
    }

    assert_eq!(
        app.world().resource::<crate::route::StageTime>().current,
        0.0
    );
    assert_eq!(
        app.world()
            .resource::<PerformanceCounters>()
            .animation_runtime_samples,
        1,
        "a new stage must be sampled even when its numeric start time matches the old stage"
    );
}

#[test]
fn composed_skeletal_fixture_samples_joint_motion() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/skel_test_composed.usda");
    let stage = Stage::open(path.to_str().expect("fixture path is valid"))
        .expect("composed skeletal fixture opens");
    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .add_plugins(bevy::transform::TransformPlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>();
    app.world_mut().insert_non_send(LiveStage::new(stage));
    for _ in 0..32 {
        app.update();
        let joint_count = {
            let world = app.world_mut();
            let mut query = world.query::<&crate::route::skel::UsdJoint>();
            query.iter(world).count()
        };
        if joint_count >= 2 {
            break;
        }
    }

    let (driver_count, rotations_authored) = {
        let world = app.world_mut();
        let mut query = world.query::<&crate::route::skel::UsdSkelAnimDriver>();
        let drivers = query.iter(world).collect::<Vec<_>>();
        (
            drivers.len(),
            drivers.iter().any(|driver| driver.has_rotations),
        )
    };
    assert!(
        driver_count > 0 && rotations_authored,
        "composed fixture must build a rotation-bearing animation driver (drivers={driver_count}, rotations={rotations_authored})"
    );
    let before = joint_transform(&mut app, "Root/Tip");
    app.world_mut().resource_mut::<StageTime>().current = 30.0;
    app.update();
    let after = joint_transform(&mut app, "Root/Tip");
    assert_ne!(before.rotation, after.rotation);
}

fn joint_transform(app: &mut App, path: &str) -> Transform {
    let world = app.world_mut();
    let mut query = world.query::<(&crate::route::skel::UsdJoint, &Transform)>();
    query
        .iter(world)
        .find(|(joint, _)| joint.path == path)
        .map(|(_, transform)| *transform)
        .unwrap_or_else(|| panic!("joint {path} was projected"))
}

#[test]
fn test_reconcile_subtrees_maintains_animated_prims_scoped_to_subtree() {
    let usda = r#"#usda 1.0
(
    startTimeCode = 0
    endTimeCode = 10
)

def Xform "World"
{
    def Xform "AnimOutside"
    {
        double3 xformOp:translate.timeSamples = {
            0: (0, 0, 0),
            10: (10, 0, 0),
        }
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }
    def Xform "A"
    {
        def Xform "AnimInsideOld"
        {
            double3 xformOp:translate.timeSamples = {
                0: (0, 0, 0),
                10: (0, 5, 0),
            }
            uniform token[] xformOpOrder = ["xformOp:translate"]
        }
    }
}
"#;

    let stage = crate::snippet::UsdSnippet::new(usda)
        .open_stage()
        .expect("animated stage opens");
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();

    let anim = app.world().resource::<AnimatedPrims>();
    assert!(anim.0.contains("/World/AnimOutside"));
    assert!(anim.0.contains("/World/A/AnimInsideOld"));

    // Remove /World/A/AnimInsideOld and define /World/A/StaticNew
    let live = app.world().get_non_send::<LiveStage>().unwrap();
    live.stage.remove_prim("/World/A/AnimInsideOld").unwrap();
    live.stage.define_prim("/World/A/StaticNew").unwrap();

    let _ = live.drain_change_batch();
    live.enqueue_resync("/World/A");
    app.update();

    let anim_after = app.world().resource::<AnimatedPrims>();
    // Unaffected outside animated path is preserved
    assert!(anim_after.0.contains("/World/AnimOutside"));
    // Old subtree animated path was cleaned
    assert!(!anim_after.0.contains("/World/A/AnimInsideOld"));
    // Static new path is not animated
    assert!(!anim_after.0.contains("/World/A/StaticNew"));
}

#[test]
fn test_reconcile_subtrees_adds_animated_prim_under_affected_subtree() {
    let usda = r#"#usda 1.0
(
    startTimeCode = 1
    endTimeCode = 10
)
def Xform "World"
{
    def Xform "A"
    {
        def Xform "StaticA" {}
    }
    def Xform "B"
    {
        def Xform "StaticB" {}
    }
}
"#;
    let stage = crate::snippet::UsdSnippet::new(usda)
        .open_stage()
        .expect("stage opens");
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();

    let anim = app.world().resource::<AnimatedPrims>();
    assert!(anim.0.is_empty());

    // Define /World/A/AnimNew with time samples
    let live = app.world().get_non_send::<LiveStage>().unwrap();
    let anim_prim = live.stage.define_prim("/World/A/AnimNew").unwrap();
    anim_prim
        .create_attribute("xformOp:translate", "double3")
        .unwrap()
        .set_at(
            openusd::sdf::Value::Vec3d(openusd::gf::Vec3d::from([0.0, 0.0, 0.0])),
            openusd::usd::TimeCode::new(1.0),
        )
        .unwrap()
        .set_at(
            openusd::sdf::Value::Vec3d(openusd::gf::Vec3d::from([0.0, 5.0, 0.0])),
            openusd::usd::TimeCode::new(2.0),
        )
        .unwrap();

    let _ = live.drain_change_batch();
    live.enqueue_resync("/World/A");
    app.update();

    let anim_after = app.world().resource::<AnimatedPrims>();
    assert!(
        anim_after.0.contains("/World/A/AnimNew"),
        "AnimatedPrims must contain newly added animated prim in subtree"
    );
    assert!(!anim_after.0.contains("/World/B/StaticB"));
}
