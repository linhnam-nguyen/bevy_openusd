use bevy::prelude::*;
use openusd::usd::Stage;
use usd_model::EntityKey;

use crate::live::{LiveStage, LiveStagePlugin, PathStore, PrimEntities, SemanticEntityIndex};

#[test]
fn compact_path_store_shares_canonical_ancestors() {
    let mut paths = PathStore::default();
    let first = paths.intern("/World/A/First");
    let second = paths.intern("/World/A/Second");

    assert_eq!(paths.intern("/World/A/First"), first);
    assert_eq!(paths.intern("/World/A/Second"), second);
    assert_eq!(
        paths.len(),
        5,
        "root and shared ancestors are interned once"
    );
    assert_eq!(
        paths.path_bytes(),
        [
            "/",
            "/World",
            "/World/A",
            "/World/A/First",
            "/World/A/Second",
        ]
        .into_iter()
        .map(str::len)
        .sum::<usize>()
    );
}

#[test]
fn prim_entities_subtree_uses_compact_topology_and_cleans_edges() {
    let mut world = World::new();
    let root = world.spawn_empty().id();
    let parent = world.spawn_empty().id();
    let child = world.spawn_empty().id();
    let sibling = world.spawn_empty().id();
    let mut paths = PathStore::default();
    let mut map = PrimEntities::default();
    map.insert(&mut paths, "/", root);
    map.insert(&mut paths, "/World/A", parent);
    map.insert(&mut paths, "/World/A/Child", child);
    map.insert(&mut paths, "/World/B", sibling);

    let subtree = map
        .subtree(&paths, "/World/A")
        .into_iter()
        .filter_map(|(path, entity)| paths.path(path).map(|path| (path.to_owned(), entity)))
        .collect::<Vec<_>>();
    assert_eq!(
        subtree,
        vec![
            ("/World/A".to_owned(), parent),
            ("/World/A/Child".to_owned(), child),
        ]
    );

    assert_eq!(map.remove_path(&paths, "/World/A/Child"), Some(child));
    assert!(map.subtree(&paths, "/World/A/Child").is_empty());
    assert_eq!(map.entity(&paths, "/World/A"), Some(parent));
    assert_eq!(map.entity(&paths, "/World/B"), Some(sibling));
}

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
        .entity(app.world().resource::<PathStore>(), "/World/B")
        .unwrap();
    let child_entity = app
        .world()
        .resource::<PrimEntities>()
        .entity(app.world().resource::<PathStore>(), "/World/A/Child")
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
