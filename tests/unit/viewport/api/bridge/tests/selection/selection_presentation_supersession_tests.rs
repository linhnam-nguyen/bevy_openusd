//! Presentation reconciliation regressions kept separate from traversal and
//! topology boundedness fixtures so each test module stays within OR8's
//! handwritten Rust source-size gate.

use super::*;
use bevy_mod_outline::OutlineVolume;
use viewport_protocol::ColorRgb8;

#[test]
fn superseded_outline_restarts_after_swap_remove_and_updates_every_live_entity() {
    const COUNT: usize = 1_000;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<SelectedTargets>()
        .init_resource::<ViewerSettingsState>()
        .init_resource::<SelectedRenderableProjection>()
        .init_resource::<SelectionPresentationPolicy>()
        .init_resource::<crate::viewport::scene::SelectionOutlineState>();
    register_selection_projection_observers(&mut app);
    app.add_systems(
        Update,
        (
            sync_selected_renderable_projection,
            crate::viewport::scene::sync_selection_outlines,
        )
            .chain(),
    );

    let root_path = "/World/OutlineChurn";
    let root = app.world_mut().spawn(UsdPrimRef::new(root_path)).id();
    let outside = app
        .world_mut()
        .spawn(UsdPrimRef::new("/World/Outside"))
        .id();
    let mut meshes = Vec::with_capacity(COUNT);
    for number in 0..COUNT {
        meshes.push(
            app.world_mut()
                .spawn((
                    UsdPrimRef::new(format!("{root_path}/Mesh{number:04}")),
                    Mesh3d(Handle::<Mesh>::default()),
                    ChildOf(root),
                ))
                .id(),
        );
    }

    *app.world_mut().resource_mut::<SceneAnchorIndex>() =
        SceneAnchorIndex::from_test_entity(anchor(root_path), root);
    app.update();
    select(&mut app, anchor(root_path));
    loop {
        app.update();
        let projection_pending = app
            .world()
            .resource::<SelectedRenderableProjection>()
            .is_pending();
        let outline_pending = app
            .world()
            .resource::<crate::viewport::scene::SelectionOutlineState>()
            .is_pending();
        if !projection_pending && !outline_pending {
            break;
        }
    }

    let expected = ColorRgb8::new(0xE1, 0x42, 0x7A);
    app.world_mut()
        .resource_mut::<ViewerSettingsState>()
        .0
        .selection
        .boundary_color = expected;

    // One update advances only a prefix of the 1k boundary reconciliation.
    app.update();
    assert!(
        app.world()
            .resource::<crate::viewport::scene::SelectionOutlineState>()
            .is_pending()
    );

    // Remove an early dense-order entry while the old job is in flight. The
    // applied EntityOrder swap-removes its last member into an already-passed
    // slot, exactly the stale-cursor case from Owner Review.
    let removed = meshes[0];
    app.world_mut().entity_mut(removed).insert(ChildOf(outside));

    for _ in 0..128 {
        app.update();
        let projection_pending = app
            .world()
            .resource::<SelectedRenderableProjection>()
            .is_pending();
        let outline_pending = app
            .world()
            .resource::<crate::viewport::scene::SelectionOutlineState>()
            .is_pending();
        if !projection_pending && !outline_pending {
            break;
        }
    }

    assert!(app.world().get::<OutlineVolume>(removed).is_none());
    let expected_rgba = [expected.r, expected.g, expected.b, 0xFF];
    for entity in meshes.into_iter().skip(1) {
        let outline = app
            .world()
            .get::<OutlineVolume>(entity)
            .expect("every live selected mesh keeps an outline");
        assert_eq!(outline.colour.to_srgba().to_u8_array(), expected_rgba);
    }
}
