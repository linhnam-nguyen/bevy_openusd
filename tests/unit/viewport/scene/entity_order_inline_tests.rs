use super::*;
use bevy::prelude::World;

#[test]
fn remove_readd_churn_never_grows_history() {
    let mut world = World::new();
    let entity = world.spawn_empty().id();
    let mut order = EntityOrder::default();

    for _ in 0..10_000 {
        assert!(order.insert(entity));
        assert!(!order.insert(entity));
        assert_eq!(order.len(), 1);
        assert!(order.remove(entity));
        assert_eq!(order.len(), 0);
    }

    assert!(order.insert(entity));
    assert_eq!(order.iter().collect::<Vec<_>>(), vec![entity]);
}

#[test]
fn swap_remove_repairs_the_moved_index() {
    let mut world = World::new();
    let first = world.spawn_empty().id();
    let middle = world.spawn_empty().id();
    let last = world.spawn_empty().id();
    let mut order = EntityOrder::default();

    assert!(order.insert(first));
    assert!(order.insert(middle));
    assert!(order.insert(last));
    assert!(order.remove(middle));
    assert_eq!(order.len(), 2);
    assert!(order.remove(last));
    assert_eq!(order.iter().collect::<Vec<_>>(), vec![first]);
}
