use super::*;

#[test]
fn semantic_index_replaces_both_sides_of_a_mapping() {
    let mut world = bevy::ecs::world::World::new();
    let first = world.spawn_empty().id();
    let second = world.spawn_empty().id();
    let old_key = EntityKey::from("application:old");
    let new_key = EntityKey::from("application:new");
    let mut index = SemanticEntityIndex::default();

    index.insert(old_key.clone(), first);
    index.insert(new_key.clone(), first);
    assert_eq!(index.entity(&old_key), None);
    assert_eq!(index.entity(&new_key), Some(first));
    assert_eq!(index.key(first), Some(&new_key));

    index.insert(new_key.clone(), second);
    assert_eq!(index.entity(&new_key), Some(second));
    assert_eq!(index.key(first), None);
    assert_eq!(index.key(second), Some(&new_key));
}
