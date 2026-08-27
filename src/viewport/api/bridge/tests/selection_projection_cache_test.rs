use std::time::Instant;

use super::*;

use bevy_glacial::prelude::BoundsGizmoTarget;
use viewport_protocol::SelectionReadModel;

use crate::viewport::scene::{
    SectionBoxGizmoTarget, SectionBoxState, SelectedRenderableProjection, SelectionOutlineState,
    sync_section_box_gizmo_target, sync_section_box_state, sync_selected_renderable_projection,
    sync_selection_outlines,
};

#[test]
fn selection_projection_reuses_unchanged_targets_and_deltas_only_touch_the_change() {
    let mut app = indexed_scene_app(3);
    app.init_resource::<SelectedRenderableProjection>()
        .init_resource::<SelectionOutlineState>()
        .add_systems(Update, sync_selected_renderable_projection)
        .add_systems(
            Update,
            sync_selection_outlines.after(sync_selected_renderable_projection),
        );
    app.update();

    set_selection(&mut app, selection(1));
    app.update();
    let projection = app.world().resource::<SelectedRenderableProjection>();
    assert_eq!(projection.resolution_count(), 1);
    assert_eq!(projection.renderables().len(), 1);
    assert_eq!(projection.added_renderables().len(), 1);
    assert!(projection.removed_renderables().is_empty());
    assert_eq!(
        app.world().resource::<SelectionOutlineState>().last_added,
        1
    );

    set_selection(&mut app, selection(2));
    app.update();
    let projection = app.world().resource::<SelectedRenderableProjection>();
    assert_eq!(projection.resolution_count(), 2);
    assert_eq!(projection.renderables().len(), 2);
    assert_eq!(projection.added_renderables().len(), 1);
    assert!(projection.removed_renderables().is_empty());
    let outlines = app.world().resource::<SelectionOutlineState>();
    assert_eq!(outlines.last_added, 1);
    assert_eq!(outlines.last_removed, 0);
    assert_eq!(outlines.last_updated, 1);

    set_selection(
        &mut app,
        SelectionReadModel {
            targets: vec![anchor(1)],
            primary: Some(anchor(1)),
        },
    );
    app.update();
    let projection = app.world().resource::<SelectedRenderableProjection>();
    assert_eq!(projection.resolution_count(), 2);
    assert_eq!(projection.renderables().len(), 1);
    assert!(projection.added_renderables().is_empty());
    assert_eq!(projection.removed_renderables().len(), 1);
    let outlines = app.world().resource::<SelectionOutlineState>();
    assert_eq!(outlines.last_added, 0);
    assert_eq!(outlines.last_removed, 1);
    assert_eq!(outlines.last_updated, 0);
}

#[test]
fn interrupted_outline_work_reconciles_entities_applied_before_selection_changes() {
    let mut app = super::projection_profile_test::combined_presentation_app(false);

    set_selection(&mut app, selection(5_000));
    app.update();
    let first_frame_outlines = app
        .world_mut()
        .query_filtered::<Entity, With<SelectionOutline>>()
        .iter(app.world())
        .count();
    assert_eq!(first_frame_outlines, 256);
    assert!(app.world().resource::<SelectionOutlineState>().is_pending());

    set_selection(
        &mut app,
        SelectionReadModel {
            targets: vec![anchor(4_999)],
            primary: Some(anchor(4_999)),
        },
    );
    for _ in 0..32 {
        app.update();
        if !app.world().resource::<SelectionOutlineState>().is_pending() {
            break;
        }
    }

    assert!(
        !app.world().resource::<SelectionOutlineState>().is_pending(),
        "replacement outline work must settle within the bounded test budget"
    );
    let outlines = app
        .world_mut()
        .query_filtered::<Entity, With<SelectionOutline>>()
        .iter(app.world())
        .collect::<Vec<_>>();
    assert_eq!(outlines.len(), 1);
    let outlined_paths = app
        .world_mut()
        .query::<(&UsdPrimRef, &SelectionOutline)>()
        .iter(app.world())
        .map(|(prim, _)| prim.path.to_owned())
        .collect::<Vec<_>>();
    assert_eq!(outlined_paths, vec!["/World/Profile04999".to_owned()]);
}

#[test]
fn section_box_projection_keeps_one_aggregate_and_one_fit_per_selection_delta() {
    let mut app = indexed_scene_app(3);
    app.init_resource::<SelectedRenderableProjection>()
        .init_resource::<SectionBoxState>();
    app.world_mut()
        .resource_mut::<ViewerSettingsState>()
        .set_section_box_enabled(true);
    app.add_systems(
        Update,
        (
            sync_selected_renderable_projection,
            sync_section_box_state,
            sync_section_box_gizmo_target,
        )
            .chain(),
    );
    app.update();

    set_selection(&mut app, selection(1));
    app.update();
    let projection = app.world().resource::<SelectedRenderableProjection>();
    let first_bounds_generation = projection.bounds_generation();
    let first_resolution_count = projection.resolution_count();
    let first_context_generation = app
        .world()
        .resource::<SectionBoxState>()
        .bounds_context_generation;
    assert_eq!(first_resolution_count, 1);
    assert!(app.world().resource::<SectionBoxState>().visible);
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<SectionBoxGizmoTarget>>()
            .iter(app.world())
            .count(),
        1
    );
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<BoundsGizmoTarget>>()
            .iter(app.world())
            .count(),
        1
    );

    set_selection(&mut app, selection(2));
    app.update();
    let projection = app.world().resource::<SelectedRenderableProjection>();
    assert_eq!(projection.resolution_count(), first_resolution_count + 1);
    assert_eq!(projection.bounds_generation(), first_bounds_generation + 1);
    assert_eq!(
        app.world()
            .resource::<SectionBoxState>()
            .bounds_context_generation,
        first_context_generation + 1
    );

    set_selection(
        &mut app,
        SelectionReadModel {
            targets: vec![anchor(1)],
            primary: Some(anchor(1)),
        },
    );
    app.update();
    let projection = app.world().resource::<SelectedRenderableProjection>();
    assert_eq!(projection.resolution_count(), first_resolution_count + 1);
    assert_eq!(projection.bounds_generation(), first_bounds_generation + 2);
    assert_eq!(
        app.world()
            .resource::<SectionBoxState>()
            .bounds_context_generation,
        first_context_generation + 2
    );
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<SectionBoxGizmoTarget>>()
            .iter(app.world())
            .count(),
        1
    );
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<BoundsGizmoTarget>>()
            .iter(app.world())
            .count(),
        1
    );

    let projection = app.world().resource::<SelectedRenderableProjection>();
    let idle_resolution_count = projection.resolution_count();
    let idle_generation = projection.generation();
    let idle_bounds_generation = projection.bounds_generation();
    let idle_context_generation = app
        .world()
        .resource::<SectionBoxState>()
        .bounds_context_generation;
    let idle_revision = app.world().resource::<SectionBoxState>().revision;
    let idle_fast_path_count = app
        .world()
        .resource::<SectionBoxState>()
        .idle_fast_path_count;
    let idle_section_resolution_count = app
        .world()
        .resource::<SectionBoxState>()
        .selection_resolution_count;
    app.update();
    assert_eq!(
        app.world()
            .resource::<SelectedRenderableProjection>()
            .resolution_count(),
        idle_resolution_count
    );
    assert_eq!(
        app.world()
            .resource::<SelectedRenderableProjection>()
            .generation(),
        idle_generation
    );
    assert_eq!(
        app.world()
            .resource::<SelectedRenderableProjection>()
            .bounds_generation(),
        idle_bounds_generation
    );
    assert_eq!(
        app.world()
            .resource::<SectionBoxState>()
            .bounds_context_generation,
        idle_context_generation
    );
    assert_eq!(
        app.world().resource::<SectionBoxState>().revision,
        idle_revision
    );
    assert_eq!(
        app.world()
            .resource::<SectionBoxState>()
            .selection_resolution_count,
        idle_section_resolution_count
    );
    assert_eq!(
        app.world()
            .resource::<SectionBoxState>()
            .idle_fast_path_count,
        idle_fast_path_count + 1
    );
}

#[test]
#[ignore = "I1.7.4 incremental projection profile; run explicitly with --ignored"]
fn profile_i1_7_4_incremental_projection_delta() {
    let mut app = super::projection_profile_test::combined_presentation_app(false);

    set_selection(&mut app, selection(4_999));
    let started = Instant::now();
    app.update();
    let initial_micros = started.elapsed().as_micros();
    let before_delta_resolution = app
        .world()
        .resource::<SelectedRenderableProjection>()
        .resolution_count();
    let before_delta_context = app
        .world()
        .resource::<SectionBoxState>()
        .bounds_context_generation;

    set_selection(&mut app, selection(5_000));
    let started = Instant::now();
    app.update();
    let delta_micros = started.elapsed().as_micros();
    let projection = app.world().resource::<SelectedRenderableProjection>();
    assert_eq!(
        projection.resolution_count(),
        before_delta_resolution + 1,
        "one added anchor must resolve once"
    );
    assert_eq!(
        app.world().resource::<SelectionOutlineState>().last_added,
        1
    );
    assert_eq!(
        app.world().resource::<SelectionOutlineState>().last_removed,
        0
    );
    assert_eq!(
        app.world()
            .resource::<SelectionColorOverrideState>()
            .last_affected_entities,
        1
    );
    assert_eq!(
        app.world()
            .resource::<SectionBoxState>()
            .bounds_context_generation,
        before_delta_context + 1
    );
    let mut steady_delta_samples = Vec::with_capacity(REPEATS * 2);
    for _ in 0..REPEATS {
        set_selection(&mut app, selection(4_999));
        let started = Instant::now();
        app.update();
        steady_delta_samples.push(started.elapsed().as_micros());
        assert_eq!(
            app.world().resource::<SelectionOutlineState>().last_removed,
            1
        );

        set_selection(&mut app, selection(5_000));
        let started = Instant::now();
        app.update();
        steady_delta_samples.push(started.elapsed().as_micros());
        assert_eq!(
            app.world().resource::<SelectionOutlineState>().last_added,
            1
        );
        assert_eq!(
            app.world()
                .resource::<SelectionColorOverrideState>()
                .last_affected_entities,
            1
        );
    }
    app.update();
    assert_eq!(
        app.world()
            .resource::<SelectionColorOverrideState>()
            .last_affected_entities,
        0
    );
    let steady_delta_max = steady_delta_samples
        .iter()
        .copied()
        .max()
        .unwrap_or_default();
    println!(
        "I1.7.4 incremental_projection selected_before=4999 selected_after=5000 initial_update_us={initial_micros} delta_update_us={delta_micros} steady_delta_median_us={} steady_delta_max_us={steady_delta_max} resolution_delta=1 outline_delta=1",
        median_micros(&mut steady_delta_samples)
    );
}

#[test]
#[ignore = "I1.7.4 projection-only profile; run explicitly with --ignored"]
fn profile_i1_7_4_projection_only() {
    let mut app = indexed_scene_app(ACCEPTED_SCENE_SIZE);
    app.init_resource::<SelectedRenderableProjection>()
        .add_systems(Update, sync_selected_renderable_projection);
    app.update();

    set_selection(&mut app, selection(4_999));
    let started = Instant::now();
    app.update();
    let initial_micros = started.elapsed().as_micros();
    let before_delta_resolution = app
        .world()
        .resource::<SelectedRenderableProjection>()
        .resolution_count();

    set_selection(&mut app, selection(5_000));
    let started = Instant::now();
    app.update();
    let delta_micros = started.elapsed().as_micros();
    let projection = app.world().resource::<SelectedRenderableProjection>();
    assert_eq!(projection.resolution_count(), before_delta_resolution + 1);
    println!(
        "I1.7.4 projection_only selected_before=4999 selected_after=5000 initial_update_us={initial_micros} delta_update_us={delta_micros} resolution_delta=1"
    );
}
