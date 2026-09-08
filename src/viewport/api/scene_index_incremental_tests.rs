use super::*;

#[test]
fn progressive_unique_prim_additions_use_incremental_index_ingestion() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<CurrentHierarchyProjection>()
        .init_resource::<Spawned>()
        .add_systems(Update, refresh_scene_anchor_index);

    let root = app
        .world_mut()
        .spawn(usd_bevy::UsdPrimRef::new("/World"))
        .id();
    app.update();
    let full_rebuilds = app.world().resource::<SceneAnchorIndex>().rebuild_count();
    assert_eq!(full_rebuilds, 1);

    for index in 0..256 {
        app.world_mut().spawn((
            usd_bevy::UsdPrimRef::new(format!("/World/Element{index:03}")),
            ChildOf(root),
        ));
    }
    app.update();

    // The first quiet update publishes the coalesced dense/protocol views.
    app.update();

    let index = app.world().resource::<SceneAnchorIndex>();
    assert_eq!(index.rebuild_count(), full_rebuilds);
    let work = index.incremental_work();
    assert_eq!(work.admitted_rows, 256);
    assert!(work.reindexed_rows >= 257);
    assert!(work.projected_rows >= 257);
    let child = SceneAnchor::active_session("/World/Element127");
    assert!(index.resolve(&child).is_some());
    assert!(
        index
            .roots_read_model()
            .prims
            .iter()
            .any(|node| node.anchor.prim_path == "/World" && node.has_children)
    );
}
