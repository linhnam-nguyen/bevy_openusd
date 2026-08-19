use bevy::prelude::*;

use crate::live::{AnimatedPrims, LiveStage, LiveStagePlugin};

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
