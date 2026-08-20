use bevy::prelude::*;
use openusd::usd::Stage;
use usd_model::EntityKey;

use crate::live::{LiveStage, LiveStagePlugin, PrimEntities, SemanticEntityIndex};

#[test]
fn test_reconcile_subtrees_maintains_semantic_entity_index() {
    let stage = Stage::builder()
        .in_memory("semantic-index-subtree.usda")
        .expect("in-memory stage");

    stage.define_prim("/World").unwrap();
    stage.define_prim("/World/A").unwrap();
    stage.define_prim("/World/A/Child").unwrap();
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
    let child_entity = app
        .world()
        .resource::<PrimEntities>()
        .entity("/World/A/Child")
        .unwrap();

    let key_b = EntityKey::new("entity_b");
    let key_child = EntityKey::new("entity_child");

    // Register semantic keys
    {
        let mut semantic_index = app.world_mut().resource_mut::<SemanticEntityIndex>();
        semantic_index.insert(key_b.clone(), world_b_entity);
        semantic_index.insert(key_child.clone(), child_entity);
    }

    // Remove /World/A/Child and enqueue resync on /World/A
    let live = app.world().get_non_send::<LiveStage>().unwrap();
    live.stage.remove_prim("/World/A/Child").unwrap();
    let _ = live.drain_change_batch();
    live.enqueue_resync("/World/A");

    app.update();

    let semantic_index = app.world().resource::<SemanticEntityIndex>();
    // Sibling outside subtree remains mapped
    assert_eq!(semantic_index.entity(&key_b), Some(world_b_entity));
    assert_eq!(semantic_index.key(world_b_entity), Some(&key_b));

    // Despawned entity mapping is completely cleaned
    assert!(semantic_index.entity(&key_child).is_none());
    assert!(semantic_index.key(child_entity).is_none());
}
