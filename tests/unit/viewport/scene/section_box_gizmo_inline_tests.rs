use super::*;
use viewport_protocol::SceneAnchor;

#[test]
fn face_drag_promotes_autofit_to_user_adjusted_and_preserves_drag_baseline() {
    let mut state = SectionBoxState::default();
    state.enabled = true;
    state.visible = true;
    state.targets = vec![SceneAnchor::active_session("/World/Box")];
    state.transform = Transform::from_scale(Vec3::splat(10.0));
    let original_revision = state.revision;

    apply_section_box_face_drag(&mut state, SectionBoxFace::PositiveX, -3.0);
    let first = state.transform;
    assert_eq!(state.pose_authority, SectionBoxPoseAuthority::UserAdjusted);
    assert_eq!(
        state.face_drag.unwrap().start_transform.scale,
        Vec3::splat(10.0)
    );
    assert_eq!(state.transform.scale.x, 7.0);
    assert_eq!(state.transform.translation.x, -1.5);
    assert_eq!(state.revision, original_revision + 1);

    apply_section_box_face_drag(&mut state, SectionBoxFace::PositiveX, -4.0);
    assert_eq!(state.transform.scale.x, 6.0);
    assert_eq!(state.transform.translation.x, -2.0);
    assert_ne!(state.transform, first);

    state.reset_geometry();
    assert_eq!(state.pose_authority, SectionBoxPoseAuthority::AutoFit);
    assert!(state.face_drag.is_none());
}

#[test]
fn face_adjusted_box_preserves_dimensions_through_translation_and_rotation() {
    let mut state = SectionBoxState::default();
    state.transform = Transform::from_scale(Vec3::splat(10.0));

    apply_section_box_face_drag(&mut state, SectionBoxFace::PositiveX, -3.0);
    let resized_scale = state.transform.scale;

    let translated = Transform {
        translation: Vec3::new(4.0, 5.0, 6.0),
        ..state.transform
    };
    apply_section_box_gizmo_transform(&mut state, translated);
    assert_eq!(state.transform.translation, translated.translation);
    assert_eq!(state.transform.scale, resized_scale);

    let rotated = Transform {
        rotation: Quat::from_rotation_y(0.5),
        ..state.transform
    };
    apply_section_box_gizmo_transform(&mut state, rotated);
    assert_eq!(state.transform.rotation, rotated.rotation);
    assert_eq!(state.transform.scale, resized_scale);
}

#[test]
fn visible_state_spawns_exactly_one_aggregate_gizmo_target() {
    let mut app = App::new();
    app.init_resource::<SectionBoxState>()
        .add_systems(Update, sync_section_box_gizmo_target);
    let mut state = app.world_mut().resource_mut::<SectionBoxState>();
    state.enabled = true;
    state.visible = true;
    state.targets = vec![SceneAnchor::active_session("/World/Box")];
    state.transform = Transform::from_scale(Vec3::splat(2.0));
    state.bounds = None;
    state.clip_planes = SectionBoxClipPlanes::default();

    app.update();
    let count = app
        .world_mut()
        .query_filtered::<Entity, With<SectionBoxGizmoTarget>>()
        .iter(app.world())
        .count();
    assert_eq!(count, 1);

    let first_entity = app
        .world_mut()
        .query_filtered::<Entity, With<SectionBoxGizmoTarget>>()
        .single(app.world())
        .expect("aggregate gizmo target exists");
    assert_eq!(
        app.world()
            .get::<BoundsGizmoTarget>(first_entity)
            .expect("bounds marker exists")
            .baseline_generation(),
        0
    );

    app.world_mut()
        .resource_mut::<SectionBoxState>()
        .bounds_context_generation = 1;

    app.update();
    let count = app
        .world_mut()
        .query_filtered::<Entity, With<SectionBoxGizmoTarget>>()
        .iter(app.world())
        .count();
    assert_eq!(count, 1);

    let second_entity = app
        .world_mut()
        .query_filtered::<Entity, With<SectionBoxGizmoTarget>>()
        .single(app.world())
        .expect("aggregate gizmo target remains");
    assert_eq!(second_entity, first_entity);
    assert_eq!(
        app.world()
            .get::<BoundsGizmoTarget>(second_entity)
            .expect("bounds marker remains")
            .baseline_generation(),
        1
    );
}
