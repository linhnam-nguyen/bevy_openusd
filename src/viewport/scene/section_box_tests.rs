use super::*;

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
}

#[test]
fn unrelated_changes_do_not_require_section_box_reconciliation() {
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
