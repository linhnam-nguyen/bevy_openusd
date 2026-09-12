use bevy::ecs::hierarchy::ChildOf;
use bevy::prelude::*;
use usd_bevy::{UsdLocalExtent, UsdPrimRef};
use viewport_protocol::{SceneAnchor, SelectionReadModel};

use crate::viewport::api::{SceneAnchorIndex, ViewerSettingsState};
use crate::viewport::scene::{
    CoarseSelectionProxyState, SelectedRenderableProjection, SelectedTargets,
    SelectionPresentationPolicy, register_selection_projection_observers,
    sync_coarse_selection_proxy, sync_selected_renderable_projection,
};

const DEEP_DEPTH: usize = 50_000;
const COARSE_CHILDREN: usize = 10_000;

fn anchor(path: &str) -> SceneAnchor {
    SceneAnchor::active_session(path)
}

fn select(app: &mut App, target: SceneAnchor) {
    app.world_mut()
        .resource_mut::<SelectedTargets>()
        .replace(SelectionReadModel {
            targets: vec![target.clone()],
            primary: Some(target),
        })
        .expect("selection satisfies the protocol invariant");
    app.update();
}

fn base_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<SelectedTargets>()
        .init_resource::<ViewerSettingsState>()
        .init_resource::<SelectedRenderableProjection>();
    register_selection_projection_observers(&mut app);
    app.add_systems(Update, sync_selected_renderable_projection);
    app
}

#[test]
fn fifty_thousand_deep_chain_budgets_enter_and_unwind_work() {
    let mut app = base_app();
    let root_path = "/World/Deep";
    let root = app
        .world_mut()
        .spawn((UsdPrimRef::new(root_path), GlobalTransform::IDENTITY))
        .id();
    let mut parent = root;
    for depth in 0..DEEP_DEPTH {
        let entity = app
            .world_mut()
            .spawn((
                UsdPrimRef::new(format!("{root_path}/N{depth:05}")),
                ChildOf(parent),
            ))
            .id();
        parent = entity;
    }
    app.world_mut()
        .entity_mut(parent)
        .insert(Mesh3d(Handle::<Mesh>::default()));

    *app.world_mut().resource_mut::<SceneAnchorIndex>() =
        SceneAnchorIndex::from_test_entity(anchor(root_path), root);
    app.update();
    select(&mut app, anchor(root_path));

    let mut updates = 0;
    let mut unwind_updates = 0;
    loop {
        app.update();
        updates += 1;
        let projection = app.world().resource::<SelectedRenderableProjection>();
        assert!(
            projection.last_cursor_work() <= 256,
            "deep hierarchy exceeded the per-update cursor budget"
        );
        if projection.renderables().len() == 1 && projection.is_pending() {
            unwind_updates += 1;
        }
        if !projection.is_pending() {
            break;
        }
        assert!(updates <= 1_000, "deep hierarchy did not converge");
    }

    let projection = app.world().resource::<SelectedRenderableProjection>();
    assert_eq!(projection.renderables().len(), 1);
    assert!(
        unwind_updates > 1,
        "terminal stack unwind must span multiple bounded updates"
    );
}

#[test]
fn repeated_reparent_churn_does_not_grow_target_order_history() {
    let mut app = base_app();
    let root_path = "/World/SelectedRoot";
    let root = app.world_mut().spawn(UsdPrimRef::new(root_path)).id();
    let outside = app
        .world_mut()
        .spawn(UsdPrimRef::new("/World/Outside"))
        .id();
    let mesh = app
        .world_mut()
        .spawn((
            UsdPrimRef::new(format!("{root_path}/Mesh")),
            Mesh3d(Handle::<Mesh>::default()),
            ChildOf(root),
        ))
        .id();

    *app.world_mut().resource_mut::<SceneAnchorIndex>() =
        SceneAnchorIndex::from_test_entity(anchor(root_path), root);
    app.update();
    select(&mut app, anchor(root_path));
    while app
        .world()
        .resource::<SelectedRenderableProjection>()
        .is_pending()
    {
        app.update();
    }

    for _ in 0..1_000 {
        app.world_mut().entity_mut(mesh).insert(ChildOf(outside));
        while app
            .world()
            .resource::<SelectedRenderableProjection>()
            .is_pending()
        {
            app.update();
            assert!(
                app.world()
                    .resource::<SelectedRenderableProjection>()
                    .last_topology_work()
                    <= 256
            );
        }
        {
            let projection = app.world().resource::<SelectedRenderableProjection>();
            assert_eq!(projection.renderables().len(), 0);
            assert_eq!(projection.target_order_len(&anchor(root_path)), 0);
        }

        app.world_mut().entity_mut(mesh).insert(ChildOf(root));
        while app
            .world()
            .resource::<SelectedRenderableProjection>()
            .is_pending()
        {
            app.update();
            assert!(
                app.world()
                    .resource::<SelectedRenderableProjection>()
                    .last_topology_work()
                    <= 256
            );
        }
        let projection = app.world().resource::<SelectedRenderableProjection>();
        assert_eq!(projection.renderables().len(), 1);
        assert_eq!(projection.target_order_len(&anchor(root_path)), 1);
    }
}

#[test]
fn automatic_coarse_proxy_appears_while_logical_selection_finishes_exactly() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<SelectedTargets>()
        .init_resource::<ViewerSettingsState>()
        .init_resource::<SelectedRenderableProjection>()
        .init_resource::<SelectionPresentationPolicy>()
        .init_resource::<CoarseSelectionProxyState>();
    register_selection_projection_observers(&mut app);
    app.add_systems(
        Update,
        (
            sync_selected_renderable_projection,
            sync_coarse_selection_proxy,
        )
            .chain(),
    );

    let root_path = "/World/CoarseGroup";
    let root = app
        .world_mut()
        .spawn((UsdPrimRef::new(root_path), GlobalTransform::IDENTITY))
        .id();
    for number in 0..COARSE_CHILDREN {
        app.world_mut().spawn((
            UsdPrimRef::new(format!("{root_path}/Mesh{number:05}")),
            Mesh3d(Handle::<Mesh>::default()),
            GlobalTransform::from(Transform::from_xyz(number as f32, 0.0, 0.0)),
            UsdLocalExtent {
                min: [-0.5, -0.5, -0.5],
                max: [0.5, 0.5, 0.5],
            },
            ChildOf(root),
        ));
    }

    *app.world_mut().resource_mut::<SceneAnchorIndex>() =
        SceneAnchorIndex::from_test_entity(anchor(root_path), root);
    app.update();
    select(&mut app, anchor(root_path));

    let mut saw_coarse_before_complete = false;
    let mut updates = 0;
    loop {
        app.update();
        updates += 1;
        let projection = app.world().resource::<SelectedRenderableProjection>();
        assert!(projection.last_cursor_work() <= 256);
        let policy = app.world().resource::<SelectionPresentationPolicy>();
        if policy.uses_coarse(projection.renderables().len())
            && projection.renderables().len() < COARSE_CHILDREN
        {
            saw_coarse_before_complete = true;
        }
        if !projection.is_pending() {
            break;
        }
        assert!(updates <= 240, "coarse selection did not converge");
    }

    assert!(saw_coarse_before_complete);
    assert_eq!(
        app.world()
            .resource::<SelectedRenderableProjection>()
            .renderables()
            .len(),
        COARSE_CHILDREN
    );
    let proxy = app.world().resource::<CoarseSelectionProxyState>();
    assert!(proxy.visible);
    assert!(proxy.bounds.is_some());
}

#[test]
fn deep_topology_re_admission_budgets_every_ancestor_hop() {
    let mut app = base_app();
    let root_path = "/World/DeepTopology";
    let root = app.world_mut().spawn(UsdPrimRef::new(root_path)).id();
    let mut parent = root;
    for depth in 0..DEEP_DEPTH {
        parent = app
            .world_mut()
            .spawn((
                UsdPrimRef::new(format!("{root_path}/N{depth:05}")),
                ChildOf(parent),
            ))
            .id();
    }
    let terminal = parent;
    app.world_mut()
        .entity_mut(terminal)
        .insert(Mesh3d(Handle::<Mesh>::default()));

    *app.world_mut().resource_mut::<SceneAnchorIndex>() =
        SceneAnchorIndex::from_test_entity(anchor(root_path), root);
    app.update();
    select(&mut app, anchor(root_path));
    while app
        .world()
        .resource::<SelectedRenderableProjection>()
        .is_pending()
    {
        app.update();
    }

    app.world_mut().entity_mut(terminal).remove::<Mesh3d>();
    while app
        .world()
        .resource::<SelectedRenderableProjection>()
        .is_pending()
    {
        app.update();
    }
    assert!(
        app.world()
            .resource::<SelectedRenderableProjection>()
            .renderables()
            .is_empty()
    );

    app.world_mut()
        .entity_mut(terminal)
        .insert(Mesh3d(Handle::<Mesh>::default()));
    let mut updates = 0;
    loop {
        app.update();
        updates += 1;
        let projection = app.world().resource::<SelectedRenderableProjection>();
        assert!(
            projection.last_topology_work() <= 256,
            "ancestor resolution escaped the topology budget"
        );
        if !projection.is_pending() {
            break;
        }
        assert!(
            updates <= 1_000,
            "deep topology re-admission did not converge"
        );
    }
    assert!(
        updates > 100,
        "50k ancestor hops must span many bounded updates rather than one hidden walk"
    );
    assert_eq!(
        app.world()
            .resource::<SelectedRenderableProjection>()
            .renderables()
            .len(),
        1
    );
}

#[path = "selection_presentation_supersession_test.rs"]
mod presentation_supersession;
