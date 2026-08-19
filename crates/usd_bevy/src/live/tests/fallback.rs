use bevy::ecs::hierarchy::ChildOf;
use bevy::prelude::*;
use openusd::usd::Stage;

use crate::live::{LiveStage, LiveStagePlugin, PrimEntities, ReconcileStats};
use crate::snippet::UsdSnippet;

#[test]
fn test_reconcile_subtrees_missing_external_parent_triggers_full_fallback() {
    let stage = Stage::builder()
        .in_memory("missing-external-parent.usda")
        .expect("in-memory stage");

    stage.define_prim("/World").unwrap();
    stage.define_prim("/World/A").unwrap();

    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();

    // Simulate corrupted map where external parent /World/A was removed from PrimEntities
    app.world_mut()
        .resource_mut::<PrimEntities>()
        .remove_path("/World/A");

    // Define a new child /World/A/B on stage
    let live = app.world().get_non_send::<LiveStage>().unwrap();
    live.stage.define_prim("/World/A/B").unwrap();
    let _ = live.drain_change_batch();
    // Enqueue resync scoped to /World/A/B
    live.enqueue_resync("/World/A/B");

    app.update();

    // Preflight detected missing external parent /World/A and aborted subtree reconcile,
    // falling back to reconcile_full which restored the complete hierarchy.
    let prim_entities = app.world().resource::<PrimEntities>();
    let a_entity = prim_entities.entity("/World/A").expect("/World/A restored");
    let b_entity = prim_entities
        .entity("/World/A/B")
        .expect("/World/A/B spawned");

    // Verify /World/A/B is child of /World/A, NOT child of stage root "/"
    let b_child_of = app.world().get::<ChildOf>(b_entity).expect("has ChildOf");
    assert_eq!(b_child_of.parent(), a_entity);
}

#[test]
fn test_reconcile_subtrees_missing_stage_root_triggers_full_fallback() {
    let stage = Stage::builder()
        .in_memory("missing-stage-root.usda")
        .expect("in-memory stage");

    stage.define_prim("/World").unwrap();

    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();

    // Simulate missing stage root "/"
    app.world_mut()
        .resource_mut::<PrimEntities>()
        .remove_path("/");

    let live = app.world().get_non_send::<LiveStage>().unwrap();
    live.stage.define_prim("/World/NewPrim").unwrap();
    let _ = live.drain_change_batch();
    live.enqueue_resync("/World/NewPrim");

    app.update();

    // Subtree reconcile falls back to full reconcile and restores "/"
    let prim_entities = app.world().resource::<PrimEntities>();
    assert!(prim_entities.entity("/").is_some());
    assert!(prim_entities.entity("/World/NewPrim").is_some());
}

#[test]
fn test_reconcile_full_resync_root_path_reconciles_entire_stage() {
    let stage = Stage::builder()
        .in_memory("full-reconcile-root.usda")
        .expect("in-memory stage");

    stage.define_prim("/World").unwrap();
    stage.define_prim("/World/A").unwrap();
    stage.define_prim("/World/B").unwrap();

    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();

    assert_eq!(app.world().resource::<PrimEntities>().len(), 4); // /, /World, /World/A, /World/B

    // Enqueue resync on "/" which directly invokes reconcile_full
    let live = app.world().get_non_send::<LiveStage>().unwrap();
    live.enqueue_resync("/");
    app.update();

    let stats = *app.world().resource::<ReconcileStats>();
    assert_eq!(stats.roots, 1);
    assert_eq!(stats.visited_stage_prims, 3); // /World, /World/A, /World/B
    assert_eq!(stats.patched_entities, 3);
    assert_eq!(stats.spawned_entities, 0);
    assert_eq!(stats.despawned_entities, 0);
}

#[test]
fn test_reconcile_invalid_raw_resync_root_chooses_full_reconcile() {
    let mut usda = String::from("#usda 1.0\n\ndef Xform \"World\"\n{\n");
    for group in ["A", "B", "C"] {
        usda.push_str(&format!("    def Xform \"{group}\"\n    {{\n"));
        for i in 0..10 {
            usda.push_str(&format!(
                "        def Xform \"{group}{i}\"\n        {{\n        }}\n"
            ));
        }
        usda.push_str("    }\n");
    }
    usda.push_str("}\n");

    let stage = UsdSnippet::new(&usda)
        .open_stage()
        .expect("synthetic wide stage opens");
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(LiveStage::new(stage));

    // Initial frame: 35 entities projected
    app.update();
    assert_eq!(app.world().resource::<PrimEntities>().len(), 35);

    // Enqueue invalid / unnormalizable resync root
    app.world()
        .get_non_send::<LiveStage>()
        .expect("live stage exists")
        .enqueue_resync("/World/B/Invalid..Path///");
    app.update();

    // Full reconcile visits all 34 stage prims, not just the 11 in subtree /World/B
    let stats = *app.world().resource::<ReconcileStats>();
    assert_eq!(stats.visited_stage_prims, 34);
    assert_eq!(stats.patched_entities, 34);
    assert_eq!(app.world().resource::<PrimEntities>().len(), 35);
}
