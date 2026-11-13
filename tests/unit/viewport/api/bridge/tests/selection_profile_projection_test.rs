use super::*;

use crate::viewport::scene::{
    SectionBoxGizmoTarget, SectionBoxState, SelectedRenderableProjection, SelectionColorMaterial,
    SelectionColorOverride, SelectionColorOverrideState, SelectionOutline, SelectionOutlineState,
    aggregate_selection_bounds, selected_renderable_entities, sync_section_box_gizmo_target,
    sync_section_box_state, sync_selected_renderable_projection, sync_selection_color_overrides,
    sync_selection_outlines,
};
use bevy::ecs::hierarchy::ChildOf;

const HIERARCHICAL_ROOTS: usize = 500;
const MESH_DESCENDANTS_PER_ROOT: usize = 8;
const HIERARCHICAL_PROFILE_SIZES: [usize; 5] = [1, 10, 100, 256, HIERARCHICAL_ROOTS];

pub(super) fn profile_gizmo_reconciliation() {
    let mut app = App::new();
    let mut state = SectionBoxState::default();
    state.enabled = true;
    state.visible = true;
    state.transform = Transform::from_scale(Vec3::splat(2.0));
    app.add_plugins(MinimalPlugins)
        .insert_resource(state)
        .add_systems(Update, sync_section_box_gizmo_target);
    let mut samples = Vec::with_capacity(REPEATS);
    for _ in 0..REPEATS {
        app.world_mut().resource_mut::<SectionBoxState>().visible = false;
        app.update();
        app.world_mut().resource_mut::<SectionBoxState>().visible = true;
        let started = Instant::now();
        app.update();
        samples.push(started.elapsed().as_micros());
    }
    let count = app
        .world_mut()
        .query_filtered::<Entity, With<SectionBoxGizmoTarget>>()
        .iter(app.world())
        .count();
    assert_eq!(count, 1);
    let maximum = samples.iter().copied().max().unwrap_or_default();
    println!(
        "I1.7.1 bounds_gizmo_target_reconciliation median_us={} max_update_us={maximum} targets={count}",
        median_micros(&mut samples)
    );
}

fn hierarchical_scene_app() -> App {
    let mut app = indexed_scene_app(0);
    for root_index in 0..HIERARCHICAL_ROOTS {
        let root_path = format!("/World/Profile{root_index:05}");
        let root = app
            .world_mut()
            .spawn(UsdPrimRef::new(root_path.clone()))
            .id();
        let subgroup_path = format!("{root_path}/Subgroup");
        let subgroup = app
            .world_mut()
            .spawn((UsdPrimRef::new(subgroup_path.clone()), ChildOf(root)))
            .id();
        for mesh_index in 0..MESH_DESCENDANTS_PER_ROOT {
            app.world_mut().spawn((
                UsdPrimRef::new(format!("{subgroup_path}/Mesh{mesh_index:02}")),
                Mesh3d(Handle::<Mesh>::default()),
                GlobalTransform::from(Transform::from_xyz(
                    (root_index * MESH_DESCENDANTS_PER_ROOT + mesh_index) as f32,
                    0.0,
                    0.0,
                )),
                UsdLocalExtent {
                    min: [-0.5, -0.5, -0.5],
                    max: [0.5, 0.5, 0.5],
                },
                ChildOf(subgroup),
            ));
        }
    }
    app.update();
    app
}

fn profile_hierarchical_renderables() {
    fn resolve(
        selection: Res<SelectedTargets>,
        index: Res<SceneAnchorIndex>,
        renderables: Query<(
            Option<&GlobalTransform>,
            Option<&Children>,
            Option<&Mesh3d>,
            Option<&bevy::camera::primitives::Aabb>,
            Option<&UsdLocalExtent>,
        )>,
        mut result: ResMut<ProfileCount>,
    ) {
        result.0 = selected_renderable_entities(&selection.0.targets, &index, &renderables).len();
    }

    for size in HIERARCHICAL_PROFILE_SIZES {
        let mut app = hierarchical_scene_app();
        app.insert_resource(ProfileCount::default())
            .add_systems(Update, resolve);
        app.update();
        let value = selection(size);
        let (micros, max_update, settle_frames) =
            repeat_selection_updates(&mut app, &value, App::update);
        let rendered_count = size * MESH_DESCENDANTS_PER_ROOT;
        assert_eq!(app.world().resource::<ProfileCount>().0, rendered_count);
        println!(
            "I1.7.1 hierarchical selected_renderable_resolution roots={size} rendered_descendants={rendered_count} median_us={micros} max_update_us={max_update} settle_max_frames={settle_frames}"
        );
    }
}

fn profile_hierarchical_bounds() {
    fn aggregate(
        selection: Res<SelectedTargets>,
        index: Res<SceneAnchorIndex>,
        renderables: Query<(
            Option<&GlobalTransform>,
            Option<&Children>,
            Option<&Mesh3d>,
            Option<&bevy::camera::primitives::Aabb>,
            Option<&UsdLocalExtent>,
        )>,
        mut result: ResMut<ProfileCount>,
    ) {
        result.0 = usize::from(
            aggregate_selection_bounds(&selection.0.targets, &index, &renderables).is_some(),
        );
    }

    for size in HIERARCHICAL_PROFILE_SIZES {
        let mut app = hierarchical_scene_app();
        app.insert_resource(ProfileCount::default())
            .add_systems(Update, aggregate);
        app.update();
        let value = selection(size);
        let (micros, max_update, settle_frames) =
            repeat_selection_updates(&mut app, &value, App::update);
        assert_eq!(app.world().resource::<ProfileCount>().0, 1);
        println!(
            "I1.7.1 hierarchical section_box_aggregate_bounds roots={size} rendered_descendants={} median_us={micros} max_update_us={max_update} settle_max_frames={settle_frames}",
            size * MESH_DESCENDANTS_PER_ROOT
        );
    }
}

pub(super) fn combined_presentation_app(hierarchical: bool) -> App {
    let mut app = if hierarchical {
        hierarchical_scene_app()
    } else {
        indexed_scene_app(ACCEPTED_SCENE_SIZE)
    };
    app.init_resource::<SelectionOutlineState>()
        .init_resource::<SectionBoxState>()
        .init_resource::<SelectedRenderableProjection>()
        .init_resource::<SelectionColorOverrideState>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<HoveredTarget>();
    let base = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    let selection_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    let hover_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    app.insert_resource(SelectionColorMaterial(selection_material))
        .insert_resource(HoverColorMaterial(hover_material));
    let meshes = app
        .world_mut()
        .query_filtered::<Entity, With<Mesh3d>>()
        .iter(app.world())
        .collect::<Vec<_>>();
    for entity in meshes {
        app.world_mut()
            .entity_mut(entity)
            .insert(MeshMaterial3d(base.clone()));
    }
    {
        let mut settings = app.world_mut().resource_mut::<ViewerSettingsState>();
        settings.0.selection.boundary_enabled = true;
        settings.0.selection.color_change_enabled = true;
        settings.set_section_box_enabled(true);
    }
    app.add_systems(
        Update,
        (
            sync_selected_renderable_projection,
            sync_selection_outlines,
            sync_selection_color_overrides,
            sync_section_box_state,
        )
            .chain(),
    );
    app.update();
    app
}

fn profile_combined_presentation(hierarchical: bool) {
    let sizes: &[usize] = if hierarchical {
        &HIERARCHICAL_PROFILE_SIZES
    } else {
        &PROFILE_SIZES
    };
    for &size in sizes {
        let mut app = combined_presentation_app(hierarchical);
        let value = selection(size);
        let (micros, max_update, settle_frames) =
            repeat_selection_updates(&mut app, &value, App::update);
        app.update();
        let rendered_count = if hierarchical {
            size * MESH_DESCENDANTS_PER_ROOT
        } else {
            size
        };
        let outline_count = app
            .world_mut()
            .query_filtered::<Entity, With<SelectionOutline>>()
            .iter(app.world())
            .count();
        let color_count = app
            .world_mut()
            .query_filtered::<Entity, With<SelectionColorOverride>>()
            .iter(app.world())
            .count();
        assert_eq!(outline_count, rendered_count);
        assert_eq!(color_count, rendered_count);
        assert_eq!(
            app.world().resource::<SectionBoxState>().targets.len(),
            size
        );
        println!(
            "I1.7.1 combined_renderer_update topology={} roots={size} rendered_descendants={rendered_count} median_us={micros} max_update_us={max_update} settle_max_frames={settle_frames}",
            if hierarchical { "hierarchical" } else { "flat" }
        );
    }
}

#[test]
#[ignore = "I1.7.1 profile checkpoint; run explicitly with --ignored"]
fn profile_i1_7_1_projection_scalability_correction() {
    profile_hierarchical_renderables();
    profile_hierarchical_bounds();
    profile_combined_presentation(false);
    profile_combined_presentation(true);
}
