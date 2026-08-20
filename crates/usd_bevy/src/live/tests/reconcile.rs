use bevy::prelude::*;
use openusd::usd::Stage;

use crate::live::{LiveStage, LiveStagePlugin, PrimEntities, ReconcileStats};

#[test]
fn reconcile_synthetic_wide_scopes_to_resynced_subtree() {
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

    let stage = crate::snippet::UsdSnippet::new(&usda)
        .open_stage()
        .expect("synthetic wide stage opens");
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(LiveStage::new(stage));

    // Initial frame performs initial project_stage
    app.update();
    assert_eq!(app.world().resource::<PrimEntities>().len(), 35);

    // Subtree resync targeting /World/B (1 root + 10 children = 11 prims)
    app.world()
        .get_non_send::<LiveStage>()
        .expect("live stage exists")
        .enqueue_resync("/World/B");
    app.update();

    let stats = *app.world().resource::<ReconcileStats>();
    assert_eq!(stats.roots, 1);
    assert_eq!(stats.visited_stage_prims, 11);
    assert_eq!(stats.patched_entities, 11);
    assert_eq!(stats.spawned_entities, 0);
    assert_eq!(stats.despawned_entities, 0);

    // All 35 entities remain mapped
    assert_eq!(app.world().resource::<PrimEntities>().len(), 35);
}

#[test]
fn reconcile_deep_overlap_minimizes_roots_and_scopes_work() {
    let stage = crate::snippet::UsdSnippet::new(
        r#"#usda 1.0

def Xform "World"
{
    def Xform "A"
    {
        def Xform "Child"
        {
            def Xform "Leaf"
            {
            }
        }
    }
    def Xform "B"
    {
    }
    def Xform "C"
    {
    }
}
"#,
    )
    .open_stage()
    .expect("deep overlap stage opens");

    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();

    let live = app.world().get_non_send::<LiveStage>().unwrap();
    live.enqueue_resync("/World/A");
    live.enqueue_resync("/World/A/Child");
    live.enqueue_resync("/World/A/Child/Leaf");
    app.update();

    let stats = *app.world().resource::<ReconcileStats>();
    // Minimizes to 1 root (/World/A) and visits/patches only 3 prims (/World/A, /World/A/Child, /World/A/Child/Leaf)
    assert_eq!(stats.roots, 1);
    assert_eq!(stats.visited_stage_prims, 3);
    assert_eq!(stats.patched_entities, 3);
    assert_eq!(stats.spawned_entities, 0);
    assert_eq!(stats.despawned_entities, 0);
}

#[test]
fn reconcile_real_materials_fixture_scopes_to_materials_subtree() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stages/materials.usda");
    let stage = Stage::open(path.to_str().expect("valid path")).expect("materials fixture opens");

    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();

    let initial_count = app.world().resource::<PrimEntities>().len();
    assert_eq!(initial_count, 13); // 12 prims + root "/"

    app.world()
        .get_non_send::<LiveStage>()
        .unwrap()
        .enqueue_resync("/World/Materials");
    app.update();

    let stats = *app.world().resource::<ReconcileStats>();
    assert_eq!(stats.roots, 1);
    assert_eq!(stats.visited_stage_prims, 7);
    assert_eq!(stats.patched_entities, 7);
    assert_eq!(stats.spawned_entities, 0);
    assert_eq!(stats.despawned_entities, 0);
}

#[test]
fn reconcile_subtree_spawns_and_despawns_while_preserving_sibling_entity_ids() {
    let stage = Stage::builder()
        .in_memory("subtree-spawn-despawn.usda")
        .expect("in-memory stage");

    stage.define_prim("/World").unwrap();
    stage.define_prim("/World/A").unwrap();
    stage.define_prim("/World/A/Child1").unwrap();
    stage.define_prim("/World/A/Child2").unwrap();
    stage.define_prim("/World/B").unwrap();

    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();

    let world_b_entity = app
        .world()
        .resource::<PrimEntities>()
        .entity("/World/B")
        .unwrap();
    let child1_entity = app
        .world()
        .resource::<PrimEntities>()
        .entity("/World/A/Child1")
        .unwrap();

    // Author changes in /World/A subtree: remove Child2, define Child3
    let live = app.world().get_non_send::<LiveStage>().unwrap();
    live.stage.remove_prim("/World/A/Child2").unwrap();
    live.stage.define_prim("/World/A/Child3").unwrap();
    let _ = live.drain_change_batch();
    live.enqueue_resync("/World/A");

    app.update();

    let stats = *app.world().resource::<ReconcileStats>();
    assert_eq!(stats.roots, 1);
    assert_eq!(stats.visited_stage_prims, 3); // /World/A, /World/A/Child1, /World/A/Child3
    assert_eq!(stats.patched_entities, 2); // /World/A, /World/A/Child1
    assert_eq!(stats.spawned_entities, 1); // /World/A/Child3
    assert_eq!(stats.despawned_entities, 1); // /World/A/Child2

    // Verify entity preservation and removal
    let prim_entities = app.world().resource::<PrimEntities>();
    assert_eq!(prim_entities.entity("/World/B"), Some(world_b_entity));
    assert_eq!(prim_entities.entity("/World/A/Child1"), Some(child1_entity));
    assert!(prim_entities.entity("/World/A/Child2").is_none());
    assert!(prim_entities.entity("/World/A/Child3").is_some());
}
