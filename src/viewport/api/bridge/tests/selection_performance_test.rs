use bevy::ecs::hierarchy::ChildOf;
use bevy::prelude::*;
use usd_bevy::{UsdLocalExtent, UsdPrimRef};
use viewport_protocol::{SceneAnchor, SelectionReadModel, ViewportCommand};

use super::support::command_test_app;
use crate::viewport::api::bridge::commands::apply_viewport_commands;
use crate::viewport::api::bridge::tree::apply_tree_commands;
use crate::viewport::api::{SceneAnchorIndex, ViewportCommandInbox, ViewportTreeCommandInbox};
use crate::viewport::scene::{
    SelectedRenderableProjection, SelectedTargets, sync_selected_renderable_projection,
};

const HEAVY_PATH: &str = "/World/HeavyGroup";
const LIGHT_PATH: &str = "/World/LightMesh";
const HEAVY_MESH_COUNT: usize = 2_048;

fn anchor(path: &str) -> SceneAnchor {
    SceneAnchor::active_session(path)
}

#[test]
fn heavy_focus_is_canceled_by_light_selection_and_control_lane_progresses() {
    let mut app = command_test_app();
    app.add_systems(Update, apply_tree_commands.after(apply_viewport_commands));

    let heavy = app.world_mut().spawn(Visibility::Visible).id();
    let light = app.world_mut().spawn(Visibility::Visible).id();
    let heavy_anchor = anchor(HEAVY_PATH);
    let light_anchor = anchor(LIGHT_PATH);
    *app.world_mut().resource_mut::<SceneAnchorIndex>() =
        SceneAnchorIndex::from_test_entities(vec![
            (heavy_anchor.clone(), heavy),
            (light_anchor.clone(), light),
        ]);

    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::ReplaceSelection {
            targets: vec![heavy_anchor.clone()],
            primary: Some(heavy_anchor.clone()),
        },
    );
    app.update();
    assert_eq!(
        app.world().resource::<SelectedTargets>().0.primary,
        Some(heavy_anchor.clone())
    );

    app.world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::FocusTarget {
            target: heavy_anchor.clone(),
            mode: viewport_protocol::FocusMode::FrameTarget,
        });
    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::ReplaceSelection {
            targets: vec![light_anchor.clone()],
            primary: Some(light_anchor.clone()),
        },
    );
    app.update();

    assert_eq!(
        app.world().resource::<SelectedTargets>().0.primary,
        Some(light_anchor.clone())
    );
    assert_eq!(
        app.world()
            .resource::<ViewportTreeCommandInbox>()
            .pending_focus_count(),
        0,
        "a focus captured against Heavy must not survive its replacement"
    );

    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetSubtreeVisibility {
            target: heavy_anchor,
            visible: false,
        },
    );
    app.update();
    assert_eq!(
        *app.world().get::<Visibility>(heavy).unwrap(),
        Visibility::Hidden
    );
}

#[test]
fn heavy_selection_releases_renderables_and_bounds_are_opt_in() {
    let mut app = projection_app();
    let heavy_anchor = anchor(HEAVY_PATH);
    let light_anchor = anchor(LIGHT_PATH);

    set_selection(&mut app, heavy_anchor.clone());
    app.update();
    let heavy_renderables = app
        .world()
        .resource::<SelectedRenderableProjection>()
        .renderables()
        .len();
    assert_eq!(heavy_renderables, HEAVY_MESH_COUNT);
    assert_eq!(
        app.world()
            .resource::<SelectedRenderableProjection>()
            .aggregate_bounds(),
        None,
        "plain selection must not derive geometry bounds"
    );

    app.world_mut()
        .resource_mut::<crate::viewport::api::ViewerSettingsState>()
        .set_section_box_enabled(true);
    app.update();
    assert!(
        app.world()
            .resource::<SelectedRenderableProjection>()
            .aggregate_bounds()
            .is_some()
    );

    set_selection(&mut app, light_anchor);
    app.update();
    let projection = app.world().resource::<SelectedRenderableProjection>();
    assert_eq!(projection.renderables().len(), 1);
    assert!(projection.aggregate_bounds().is_some());

    app.world_mut()
        .resource_mut::<SelectedTargets>()
        .clear()
        .expect("clearing the synthetic selection must succeed");
    app.update();
    assert!(
        app.world()
            .resource::<SelectedTargets>()
            .0
            .targets
            .is_empty()
    );
    assert!(
        app.world()
            .resource::<SelectedRenderableProjection>()
            .renderables()
            .is_empty()
    );
    app.world_mut()
        .resource_mut::<crate::viewport::api::ViewerSettingsState>()
        .set_section_box_enabled(false);
    app.update();
    assert_eq!(
        app.world()
            .resource::<SelectedRenderableProjection>()
            .aggregate_bounds(),
        None
    );
}

fn set_selection(app: &mut App, target: SceneAnchor) {
    app.world_mut()
        .resource_mut::<SelectedTargets>()
        .replace(SelectionReadModel {
            targets: vec![target.clone()],
            primary: Some(target),
        })
        .expect("synthetic selection must satisfy the protocol invariant");
}

fn projection_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<SelectedTargets>()
        .init_resource::<crate::viewport::api::ViewerSettingsState>()
        .init_resource::<SelectedRenderableProjection>();

    let heavy = app.world_mut().spawn(UsdPrimRef::new(HEAVY_PATH)).id();
    let mut entries = vec![(anchor(HEAVY_PATH), heavy)];
    for index in 0..HEAVY_MESH_COUNT {
        app.world_mut().spawn((
            UsdPrimRef::new(format!("{HEAVY_PATH}/Mesh{index:04}")),
            Mesh3d(Handle::<Mesh>::default()),
            GlobalTransform::from(Transform::from_xyz(index as f32, 0.0, 0.0)),
            UsdLocalExtent {
                min: [-0.5, -0.5, -0.5],
                max: [0.5, 0.5, 0.5],
            },
            ChildOf(heavy),
        ));
    }
    let light = app
        .world_mut()
        .spawn((
            UsdPrimRef::new(LIGHT_PATH),
            Mesh3d(Handle::<Mesh>::default()),
            GlobalTransform::from(Transform::from_xyz(1.0, 0.0, 0.0)),
            UsdLocalExtent {
                min: [-0.5, -0.5, -0.5],
                max: [0.5, 0.5, 0.5],
            },
        ))
        .id();
    entries.push((anchor(LIGHT_PATH), light));
    *app.world_mut().resource_mut::<SceneAnchorIndex>() =
        SceneAnchorIndex::from_test_entities(entries);
    app.add_systems(Update, sync_selected_renderable_projection);
    app.update();
    app
}
