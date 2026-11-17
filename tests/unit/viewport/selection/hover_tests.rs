use super::*;

#[test]
fn idle_pick_state_requires_no_new_input_or_camera_revision() {
    let input = ViewportNavigationInput::default();
    let mut state = HoveredTarget {
        last_pointer_position: input.pointer_position,
        last_viewport_size: Some(input.viewport_size),
        last_camera_local: Some(Transform::IDENTITY),
        last_camera_global: Some(Transform::IDENTITY),
        last_clip_from_view: Some(Mat4::IDENTITY),
        last_focused: Some(input.focused),
        last_scene_revision: Some(4),
        ..default()
    };

    assert_eq!(state.last_pointer_position, input.pointer_position);
    assert_eq!(state.last_viewport_size, Some(input.viewport_size));
    assert_eq!(state.last_scene_revision, Some(4));

    state.last_pointer_position = Some(Vec2::new(4.0, 2.0));
    assert_ne!(state.last_pointer_position, input.pointer_position);
}

#[test]
fn hover_pick_stats_start_without_raycasts() {
    assert_eq!(HoverPickStats::default().raycasts, 0);
    assert_eq!(HoverPickStats::default().skipped_idle_frames, 0);
}

#[test]
fn remote_css_pointer_maps_to_actual_camera_viewport() {
    let mapped = map_pointer_to_camera_viewport(
        Vec2::new(480.0, 270.0),
        Vec2::new(960.0, 540.0),
        Vec2::new(1920.0, 1080.0),
    );
    assert!(mapped.abs_diff_eq(Vec2::new(960.0, 540.0), 1e-5));
}

#[test]
fn remote_css_pointer_maps_non_two_x_camera_viewport() {
    let mapped = map_pointer_to_camera_viewport(
        Vec2::new(375.0, 225.0),
        Vec2::new(1000.0, 600.0),
        Vec2::new(1500.0, 900.0),
    );
    assert!(mapped.abs_diff_eq(Vec2::new(562.5, 337.5), 1e-5));
}
