use super::*;

#[test]
fn progressive_unique_prim_additions_use_incremental_index_ingestion() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<CurrentHierarchyProjection>()
        .init_resource::<Spawned>()
        .add_systems(Update, refresh_scene_anchor_index);
    register_scene_index_observers(&mut app);

    let root = app
        .world_mut()
        .spawn(usd_bevy::UsdPrimRef::new("/World"))
        .id();
    app.update();
    let full_rebuilds = app.world().resource::<SceneAnchorIndex>().rebuild_count();
    assert_eq!(full_rebuilds, 0);

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
    assert_eq!(work.admitted_rows, 257);
    assert_eq!(work.derived_flushes, 1);
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
#[test]
fn ten_thousand_prim_additions_are_admitted_with_a_hard_per_update_cap() {
    const COUNT: usize = 10_000;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<CurrentHierarchyProjection>()
        .init_resource::<Spawned>()
        .add_systems(Update, refresh_scene_anchor_index);
    register_scene_index_observers(&mut app);

    let root = app
        .world_mut()
        .spawn(usd_bevy::UsdPrimRef::new("/World"))
        .id();
    app.update();
    let rebuilds = app.world().resource::<SceneAnchorIndex>().rebuild_count();

    for number in 0..COUNT {
        app.world_mut().spawn((
            usd_bevy::UsdPrimRef::new(format!("/World/Bulk{number:05}")),
            ChildOf(root),
        ));
    }

    let mut updates = 0;
    loop {
        app.update();
        updates += 1;
        let index = app.world().resource::<SceneAnchorIndex>();
        assert!(
            index.last_refresh_admitted() <= SCENE_INDEX_ADMISSION_BUDGET,
            "Scene-index admission exceeded the hard update budget"
        );
        if index.pending_addition_count() == 0 {
            break;
        }
        assert!(updates <= 64, "bounded Scene-index queue did not drain");
    }

    // One settled update publishes the dense/protocol projection.
    app.update();
    let index = app.world().resource::<SceneAnchorIndex>();
    assert_eq!(index.rebuild_count(), rebuilds);
    assert_eq!(index.incremental_work().admitted_rows, (COUNT + 1) as u64);
    assert_eq!(index.incremental_work().derived_flushes, 1);
    assert!(
        index
            .resolve(&SceneAnchor::active_session("/World/Bulk09999"))
            .is_some()
    );
}

#[test]
fn ten_thousand_prim_full_reconciliation_capture_is_hard_capped() {
    const COUNT: usize = 10_000;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<CurrentHierarchyProjection>()
        .init_resource::<Spawned>()
        .add_systems(Update, refresh_scene_anchor_index);
    register_scene_index_observers(&mut app);

    let root = app
        .world_mut()
        .spawn(usd_bevy::UsdPrimRef::new("/World"))
        .id();
    let mut entities = Vec::with_capacity(COUNT);
    for number in 0..COUNT {
        entities.push(
            app.world_mut()
                .spawn((
                    usd_bevy::UsdPrimRef::new(format!("/World/Reconcile{number:05}")),
                    ChildOf(root),
                    Visibility::Visible,
                ))
                .id(),
        );
    }

    for _ in 0..128 {
        app.update();
        if app
            .world()
            .resource::<SceneAnchorIndex>()
            .pending_addition_count()
            == 0
        {
            break;
        }
    }
    app.update();
    let rebuilds_before = app.world().resource::<SceneAnchorIndex>().rebuild_count();

    for entity in entities {
        app.world_mut()
            .entity_mut(entity)
            .insert(Visibility::Hidden);
    }

    let mut capture_updates = 0;
    let mut saw_pending = false;
    for _ in 0..2_048 {
        app.update();
        std::thread::yield_now();
        let index = app.world().resource::<SceneAnchorIndex>();
        assert!(
            index.last_reconcile_capture_work() <= SCENE_INDEX_ADMISSION_BUDGET,
            "whole-index reconciliation exceeded the per-update capture budget"
        );
        saw_pending |= index.reconcile_is_pending();
        if saw_pending && !index.reconcile_is_pending() {
            break;
        }
        if index.last_reconcile_capture_work() > 0 {
            capture_updates += 1;
        }
    }

    let index = app.world().resource::<SceneAnchorIndex>();
    assert!(
        saw_pending,
        "bulk mutation never entered bounded reconciliation"
    );
    assert!(
        !index.reconcile_is_pending(),
        "bounded reconciliation did not publish"
    );
    assert!(
        capture_updates > 1,
        "10k full reconciliation must span multiple bounded capture updates"
    );
    assert_eq!(index.rebuild_count(), rebuilds_before + 1);
    assert_eq!(
        index.visibility_for_anchor(&SceneAnchor::active_session("/World/Reconcile09999")),
        viewport_protocol::HierarchyVisibilityState::Hidden
    );
}

#[test]
fn initial_authority_ingestion_is_bounded_and_converges_without_virtual_root_capacity() {
    const COUNT: usize = 513;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<CurrentHierarchyProjection>()
        .init_resource::<Spawned>()
        .add_systems(Update, refresh_scene_anchor_index);
    register_scene_index_observers(&mut app);

    app.world_mut().spawn(usd_bevy::UsdPrimRef::new("/"));
    let root = app
        .world_mut()
        .spawn(usd_bevy::UsdPrimRef::new("/World"))
        .id();
    for number in 0..COUNT {
        app.world_mut().spawn((
            usd_bevy::UsdPrimRef::new(format!("/World/Initial{number:04}")),
            ChildOf(root),
        ));
    }

    let mut updates = 0;
    loop {
        app.update();
        updates += 1;
        let index = app.world().resource::<SceneAnchorIndex>();
        assert!(
            index.last_refresh_admitted() <= SCENE_INDEX_ADMISSION_BUDGET,
            "initial Scene-index admission exceeded the hard update budget"
        );
        if index
            .resolve(&SceneAnchor::active_session("/World/Initial0512"))
            .is_some()
        {
            break;
        }
        assert!(updates <= 8, "bounded initial Scene-index ingestion did not converge");
    }

    app.update();
    let index = app.world().resource::<SceneAnchorIndex>();
    assert_eq!(index.incremental_work().admitted_rows, (COUNT + 1) as u64);
    assert_eq!(index.roots_read_model().total_prims, (COUNT + 1) as u32);
    assert_eq!(index.last_refresh_admitted(), 0);
    assert!(index.resolve(&SceneAnchor::active_session("/World")).is_some());
    assert!(
        index
            .resolve(&SceneAnchor::active_session("/"))
            .is_none(),
        "virtual root must not consume a Scene-index entity slot"
    );

    // A renderer-only child attachment is projection-only churn and must not
    // restart the bounded authority ingestion or a whole-index rebuild.
    let rebuilds = index.rebuild_count();
    app.world_mut().spawn(ChildOf(root));
    app.update();
    let index = app.world().resource::<SceneAnchorIndex>();
    assert_eq!(index.rebuild_count(), rebuilds);
    assert!(index.last_refresh_admitted() <= SCENE_INDEX_ADMISSION_BUDGET);
}
