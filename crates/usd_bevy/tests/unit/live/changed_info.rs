use bevy::prelude::*;

use crate::live::{
    LiveRevision, LiveStage, LiveStagePlugin, PathStore, PerformanceCounters, PrimEntities,
    StageChange, StageChangeBatch, apply_change_batch, author_transform,
};
use crate::snippet::UsdSnippet;

#[test]
fn test_bare_prim_changed_info_repatches_transform() {
    let usda = r#"#usda 1.0
def Xform "World"
{
    def Xform "A"
    {
        double3 xformOp:translate = (0, 0, 0)
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }
}

"#;
    let stage = UsdSnippet::new(usda).open_stage().expect("stage opens");
    let live = LiveStage::new(stage);

    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(live);
    app.update();

    let entity = app
        .world()
        .resource::<PrimEntities>()
        .entity(app.world().resource::<PathStore>(), "/World/A")
        .unwrap();
    let initial_transform = *app.world().get::<Transform>(entity).unwrap();
    assert_eq!(initial_transform.translation, Vec3::ZERO);

    // Author a new translation on /World/A
    {
        let live = app.world().get_non_send::<LiveStage>().unwrap();
        author_transform(
            &live.stage,
            "/World/A",
            &Transform::from_translation(Vec3::new(10.0, 20.0, 30.0)),
        )
        .unwrap();
        let _ = live.drain_change_batch();
    }

    // Manually submit a bare prim changed_info (no .property suffix)
    let live = app.world_mut().remove_non_send::<LiveStage>().unwrap();
    let mut map = app.world_mut().remove_resource::<PrimEntities>().unwrap();
    let batch = StageChangeBatch {
        revision: LiveRevision(2),
        changes: vec![StageChange {
            resynced: Vec::new(),
            changed_info: vec!["/World/A".to_string()],
        }],
    };
    apply_change_batch(app.world_mut(), &live, &mut map, &batch);
    app.world_mut().insert_non_send(live);
    app.world_mut().insert_resource(map);

    // Verify Transform component was repatched to the new translation
    let updated_transform = *app.world().get::<Transform>(entity).unwrap();
    assert_eq!(
        updated_transform.translation,
        Vec3::new(10.0, 20.0, 30.0),
        "bare prim changed_info must repatch transform"
    );
}

#[test]
fn same_stage_metadata_patch_rebuilds_hierarchy_index_for_batch_revision() -> anyhow::Result<()> {
    let stage = UsdSnippet::new(
        r#"#usda 1.0
(defaultPrim = "World")
def Xform "World"
{
    def Xform "Source" {}
}
"#,
    )
    .open_stage()?;
    stage.prim("/World/Source").set_metadata(
        "ui:displayName",
        openusd::sdf::Value::String("Before patch".to_owned()),
    )?;
    let live = LiveStage::new(stage);
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(live);
    app.update();

    let entity = app
        .world()
        .resource::<PrimEntities>()
        .entity(app.world().resource::<PathStore>(), "/World/Source")
        .expect("source prim is projected");
    assert_eq!(
        app.world().get::<crate::UsdDisplayName>(entity),
        Some(&crate::UsdDisplayName("Before patch".to_owned()))
    );

    {
        let live = app.world().get_non_send::<LiveStage>().expect("live stage");
        live.stage.prim("/World/Source").set_metadata(
            "ui:displayName",
            openusd::sdf::Value::String("After patch".to_owned()),
        )?;
    }
    app.update();

    assert_eq!(
        app.world().get::<crate::UsdDisplayName>(entity),
        Some(&crate::UsdDisplayName("After patch".to_owned()))
    );
    Ok(())
}

#[test]
fn compact_change_plan_deduplicates_paths_and_properties() {
    let usda = r#"#usda 1.0
def Xform "World"
{
    def Xform "A"
    {
        double3 xformOp:translate = (0, 0, 0)
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }
}
"#;
    let stage = UsdSnippet::new(usda).open_stage().expect("stage opens");
    let live = LiveStage::new(stage);
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(live);
    app.update();
    app.world_mut()
        .resource_mut::<PerformanceCounters>()
        .enabled = true;
    app.world_mut()
        .resource_mut::<PerformanceCounters>()
        .reset();

    let live = app.world_mut().remove_non_send::<LiveStage>().unwrap();
    let mut map = app.world_mut().remove_resource::<PrimEntities>().unwrap();
    let batch = StageChangeBatch {
        revision: LiveRevision(2),
        changes: vec![StageChange {
            resynced: Vec::new(),
            changed_info: vec![
                "/World/A.xformOp:translate".to_string(),
                "/World/A.xformOp:translate".to_string(),
                "/World/A.xformOpOrder".to_string(),
                "/World/A.xformOpOrder".to_string(),
            ],
        }],
    };
    apply_change_batch(app.world_mut(), &live, &mut map, &batch);
    app.world_mut().insert_non_send(live);
    app.world_mut().insert_resource(map);

    let counters = app.world().resource::<PerformanceCounters>();
    assert_eq!(counters.reconcile_changed_properties, 4);
    assert_eq!(counters.reconcile_distinct_prims, 1);
    assert_eq!(counters.reconcile_dependency_queries, 1);
    assert_eq!(counters.reconcile_string_materializations, 0);
}
