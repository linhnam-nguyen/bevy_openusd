use super::*;

#[test]
fn test_semantic_test_app_initialization() {
    let mut app = semantic_test_app();
    app.update();
    assert!(app.world().contains_resource::<usd_bevy::PrimEntities>());
    assert!(app.world().contains_resource::<usd_bevy::PathStore>());
}

#[test]
fn test_sorted_paths() {
    let mut paths = usd_bevy::PathStore::default();
    let mut map = usd_bevy::PrimEntities::default();
    let mut app = App::new();
    let e1 = app.world_mut().spawn_empty().id();
    let e2 = app.world_mut().spawn_empty().id();
    let e3 = app.world_mut().spawn_empty().id();
    map.insert(&mut paths, "/World/Z", e1);
    map.insert(&mut paths, "/World/A", e2);
    map.insert(&mut paths, "/World/M", e3);

    let sorted = sorted_paths(&map, &paths);
    assert_eq!(sorted, vec!["/World/A", "/World/M", "/World/Z"]);
}
