use super::*;

#[test]
fn pointer_button_transitions_cover_press_release_and_middle_mapping() {
    let previous = PointerButtons {
        primary: true,
        secondary: false,
        auxiliary: true,
    };
    let current = PointerButtons {
        primary: false,
        secondary: true,
        auxiliary: false,
    };

    assert_eq!(
        pointer_button_transition(previous.primary, current.primary),
        Some(PointerTransition::Release)
    );
    assert_eq!(
        pointer_button_transition(previous.secondary, current.secondary),
        Some(PointerTransition::Press)
    );
    assert_eq!(
        pointer_button_transition(previous.auxiliary, current.auxiliary),
        Some(PointerTransition::Release)
    );
}

#[test]
fn remote_css_pointer_maps_to_two_x_offscreen_target() {
    let mapped = normalize_pointer_position(
        Vec2::new(480.0, 270.0),
        Vec2::new(960.0, 540.0),
        UVec2::new(1920, 1080),
    );

    assert!(mapped.abs_diff_eq(Vec2::new(960.0, 540.0), 1e-5));
}

#[test]
fn remote_css_pointer_maps_to_non_two_x_offscreen_target() {
    let mapped = normalize_pointer_position(
        Vec2::new(320.0, 180.0),
        Vec2::new(1280.0, 720.0),
        UVec2::new(1920, 1080),
    );

    assert!(mapped.abs_diff_eq(Vec2::new(480.0, 270.0), 1e-5));
}

#[test]
fn remote_css_pointer_is_clamped_to_offscreen_target() {
    let mapped = normalize_pointer_position(
        Vec2::new(1400.0, -20.0),
        Vec2::new(1280.0, 720.0),
        UVec2::new(1920, 1080),
    );

    assert_eq!(mapped, Vec2::new(1920.0, 0.0));
}
