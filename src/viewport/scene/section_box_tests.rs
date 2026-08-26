use super::*;

use crate::viewport::scene::SelectedTargets;

fn bounds(min: Vec3, max: Vec3) -> SectionBoxBounds {
    SectionBoxBounds { min, max }
}

#[test]
fn aggregate_bounds_contain_every_selected_renderable() {
    let mut aggregate = bounds(Vec3::splat(1.0), Vec3::splat(2.0));
    aggregate.include(bounds(Vec3::splat(-4.0), Vec3::splat(-3.0)));
    aggregate.include(bounds(Vec3::splat(5.0), Vec3::splat(9.0)));

    assert!(aggregate.contains(bounds(Vec3::splat(1.0), Vec3::splat(2.0))));
    assert!(aggregate.contains(bounds(Vec3::splat(-4.0), Vec3::splat(-3.0))));
    assert!(aggregate.contains(bounds(Vec3::splat(5.0), Vec3::splat(9.0))));
    assert_eq!(aggregate.min, Vec3::splat(-4.0));
    assert_eq!(aggregate.max, Vec3::splat(9.0));
}

#[test]
fn fit_produces_one_box_transform_and_six_planes() {
    let fitted = bounds(Vec3::new(-2.0, -1.0, 3.0), Vec3::new(0.0, 0.0, 6.0));
    let transform = fit_transform(fitted);
    let planes = SectionBoxClipPlanes::from_bounds(fitted);

    assert_eq!(transform.translation, Vec3::new(-1.0, -0.5, 4.5));
    assert_eq!(transform.scale, Vec3::new(2.0, 1.0, 3.0));
    assert_eq!(planes.planes.len(), 6);
}

#[test]
fn user_adjusted_pose_survives_reference_bounds_update() {
    let adjusted_transform = Transform::from_xyz(8.0, 9.0, 10.0)
        .with_rotation(Quat::from_rotation_y(0.5))
        .with_scale(Vec3::splat(2.0));
    let adjusted_planes = SectionBoxClipPlanes::from_transform(adjusted_transform);
    let next = next_section_box_pose(
        adjusted_transform,
        adjusted_planes,
        SectionBoxPoseAuthority::UserAdjusted,
        true,
        Some(bounds(Vec3::splat(-5.0), Vec3::splat(5.0))),
        false,
    );

    assert_eq!(next.authority, SectionBoxPoseAuthority::UserAdjusted);
    assert_eq!(next.transform, adjusted_transform);
    assert_eq!(next.clip_planes, adjusted_planes);
}

#[test]
fn selection_change_refits_user_adjusted_pose() {
    let adjusted_transform = Transform::from_xyz(8.0, 9.0, 10.0).with_scale(Vec3::splat(2.0));
    let next = next_section_box_pose(
        adjusted_transform,
        SectionBoxClipPlanes::from_transform(adjusted_transform),
        SectionBoxPoseAuthority::UserAdjusted,
        true,
        Some(bounds(Vec3::ZERO, Vec3::splat(4.0))),
        true,
    );

    assert_eq!(next.authority, SectionBoxPoseAuthority::AutoFit);
    assert_eq!(next.transform.translation, Vec3::splat(2.0));
    assert_eq!(next.transform.scale, Vec3::splat(4.0));
}

#[test]
fn oriented_transform_produces_six_inside_facing_planes() {
    let transform = Transform {
        translation: Vec3::new(3.0, 4.0, 5.0),
        rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        scale: Vec3::new(2.0, 4.0, 6.0),
    };
    let planes = SectionBoxClipPlanes::from_transform(transform);
    let center = transform.translation.extend(1.0);

    assert_eq!(planes.planes.len(), 6);
    assert!(
        planes
            .planes
            .iter()
            .all(|plane| plane.dot(center) >= -f32::EPSILON)
    );
}

#[test]
fn empty_geometry_resets_derived_state_without_authored_geometry() {
    let mut state = SectionBoxState {
        enabled: true,
        visible: true,
        targets: vec![SceneAnchor::active_session("/World/Box")],
        transform: Transform::from_xyz(1.0, 2.0, 3.0),
        bounds: Some(bounds(Vec3::ZERO, Vec3::ONE)),
        clip_planes: SectionBoxClipPlanes::from_bounds(bounds(Vec3::ZERO, Vec3::ONE)),
        ..default()
    };
    state.reset_geometry();

    assert!(!state.visible);
    assert_eq!(state.bounds, None);
    assert_eq!(state.transform, Transform::IDENTITY);
    assert_eq!(state.pose_authority, SectionBoxPoseAuthority::AutoFit);
}

#[test]
fn disabling_resets_user_adjustment_and_reenable_refits() {
    let adjusted_transform = Transform::from_xyz(8.0, 9.0, 10.0)
        .with_rotation(Quat::from_rotation_y(0.5))
        .with_scale(Vec3::splat(2.0));
    let adjusted_planes = SectionBoxClipPlanes::from_transform(adjusted_transform);
    let disabled = next_section_box_pose(
        adjusted_transform,
        adjusted_planes,
        SectionBoxPoseAuthority::UserAdjusted,
        false,
        Some(bounds(Vec3::splat(-5.0), Vec3::splat(5.0))),
        false,
    );

    assert_eq!(disabled.authority, SectionBoxPoseAuthority::AutoFit);
    assert_eq!(disabled.transform, Transform::IDENTITY);
    assert_eq!(disabled.clip_planes, SectionBoxClipPlanes::default());

    let reenabled = next_section_box_pose(
        disabled.transform,
        disabled.clip_planes,
        disabled.authority,
        true,
        Some(bounds(Vec3::ZERO, Vec3::splat(4.0))),
        true,
    );
    assert_eq!(reenabled.authority, SectionBoxPoseAuthority::AutoFit);
    assert_eq!(reenabled.transform.translation, Vec3::splat(2.0));
    assert_eq!(reenabled.transform.scale, Vec3::splat(4.0));
}

#[test]
fn unrelated_changes_do_not_require_section_box_reconciliation() {
    // Material-only changes do not affect renderable bounds and never enter
    // the relevant_bounds_changed signal consumed by this coordinator.
    assert!(!should_reconcile_section_box(
        false, false, false, false, false, false
    ));
    assert!(should_reconcile_section_box(
        false, false, false, true, false, false
    ));
    assert!(should_reconcile_section_box(
        false, false, false, false, false, true
    ));
}

#[test]
fn material_only_component_change_does_not_reconcile_section_box() {
    let mut app = App::new();
    app.init_resource::<Assets<StandardMaterial>>();
    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    let entity = app
        .world_mut()
        .spawn((SectionBoxTrackedRenderable, MeshMaterial3d(material)))
        .id();

    let mut state = SectionBoxState::default();
    state.tracked_renderables.insert(entity);
    app.insert_resource(ViewerSettingsState::default())
        .insert_resource(SelectedTargets::default())
        .insert_resource(SceneAnchorIndex::default())
        .insert_resource(state)
        .add_systems(Update, sync_section_box_state);
    app.update();

    let replacement = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    app.world_mut()
        .entity_mut(entity)
        .insert(MeshMaterial3d(replacement));
    app.update();

    let state = app.world().resource::<SectionBoxState>();
    assert_eq!(state.revision, 0);
    assert!(state.tracked_renderables.contains(&entity));
    assert!(
        app.world()
            .get::<SectionBoxTrackedRenderable>(entity)
            .is_some()
    );
}
