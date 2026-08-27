use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use usd_bevy::UsdPrimRef;
use viewport_protocol::SelectionReadModel;

use super::{anchor, set_selection};
use crate::viewport::scene::{
    SelectionBaseMaterial, SelectionColorMaterial, SelectionColorOverride,
    SelectionColorOverrideState,
};

const SELECTION_SIZE: usize = 1_000;

fn range_selection(start: usize) -> SelectionReadModel {
    let targets = (start..start + SELECTION_SIZE)
        .map(anchor)
        .collect::<Vec<_>>();
    SelectionReadModel {
        primary: targets.first().cloned(),
        targets,
    }
}

fn material_handles(app: &mut App) -> (Handle<StandardMaterial>, Handle<StandardMaterial>) {
    let base = app
        .world_mut()
        .query::<&MeshMaterial3d<StandardMaterial>>()
        .iter(app.world())
        .next()
        .expect("profile scene must contain a mesh material")
        .0
        .clone();
    let selection = app.world().resource::<SelectionColorMaterial>().0.clone();
    (base, selection)
}

fn settle_color_work(app: &mut App) {
    for _ in 0..64 {
        if !app
            .world()
            .resource::<SelectionColorOverrideState>()
            .is_pending()
        {
            return;
        }
        app.update();
    }
    panic!("selection color work did not settle within the bounded test budget");
}

fn assert_only_range_is_selected(
    app: &mut App,
    expected_start: usize,
    base_handle: &Handle<StandardMaterial>,
    selection_handle: &Handle<StandardMaterial>,
) {
    let mut selected_count = 0;
    let mut meshes = app.world_mut().query::<(
        &UsdPrimRef,
        &MeshMaterial3d<StandardMaterial>,
        Option<&SelectionColorOverride>,
        Option<&SelectionBaseMaterial>,
    )>();
    for (prim, material, marker, base) in meshes.iter(app.world()) {
        let index = prim
            .path
            .strip_prefix("/World/Profile")
            .expect("profile mesh path")
            .parse::<usize>()
            .expect("profile mesh index");
        let expected = (expected_start..expected_start + SELECTION_SIZE).contains(&index);
        if expected {
            selected_count += 1;
            assert!(
                marker.is_some(),
                "selected mesh must retain its override marker"
            );
            assert!(
                base.is_some(),
                "selected mesh must retain its base material"
            );
            assert_eq!(material.0, *selection_handle);
        } else {
            assert!(
                marker.is_none(),
                "unselected mesh retained an override marker"
            );
            assert!(
                base.is_none(),
                "unselected mesh retained base-material ownership"
            );
            assert_ne!(material.0, *selection_handle);
            assert_eq!(material.0, *base_handle);
        }
    }
    assert_eq!(selected_count, SELECTION_SIZE);
}

#[test]
fn interrupted_selection_color_reconciles_a_to_b_and_restores_base_materials() {
    let mut app = super::projection_profile_test::combined_presentation_app(false);
    let (base_handle, selection_handle) = material_handles(&mut app);

    set_selection(&mut app, range_selection(0));
    app.update();
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<SelectionColorOverride>>()
            .iter(app.world())
            .count(),
        256
    );
    assert!(
        app.world()
            .resource::<SelectionColorOverrideState>()
            .is_pending()
    );

    set_selection(&mut app, range_selection(2_000));
    app.update();
    settle_color_work(&mut app);
    assert_only_range_is_selected(&mut app, 2_000, &base_handle, &selection_handle);
}

#[test]
fn repeated_superseding_selection_color_work_leaves_only_the_final_range_owned() {
    let mut app = super::projection_profile_test::combined_presentation_app(false);
    let (base_handle, selection_handle) = material_handles(&mut app);

    set_selection(&mut app, range_selection(0));
    app.update();
    set_selection(&mut app, range_selection(2_000));
    app.update();
    assert!(
        app.world()
            .resource::<SelectionColorOverrideState>()
            .is_pending()
    );
    set_selection(&mut app, range_selection(4_000));
    app.update();
    settle_color_work(&mut app);

    assert_only_range_is_selected(&mut app, 4_000, &base_handle, &selection_handle);
}

#[test]
fn reverting_to_the_last_completed_selection_reconciles_partial_color_work() {
    let mut app = super::projection_profile_test::combined_presentation_app(false);
    let (base_handle, selection_handle) = material_handles(&mut app);

    set_selection(&mut app, range_selection(0));
    app.update();
    set_selection(&mut app, range_selection(2_000));
    app.update();
    assert!(
        app.world()
            .resource::<SelectionColorOverrideState>()
            .is_pending()
    );
    set_selection(&mut app, range_selection(0));
    app.update();
    settle_color_work(&mut app);

    assert_only_range_is_selected(&mut app, 0, &base_handle, &selection_handle);
}
