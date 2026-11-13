use super::*;

#[test]
fn new_stream_generation_rejects_old_pointer_mapping() {
    let mut input = ViewportNavigationInput::with_viewport_size(1280, 720);
    input.begin_stream_generation(4);
    input.apply_pointer_motion(PointerMotion {
        sequence: 1,
        x_css_pixels: 40.0,
        y_css_pixels: 30.0,
        dx_css_pixels: 12.0,
        dy_css_pixels: 8.0,
        wheel_x: 0.0,
        wheel_y: 0.0,
        viewport_css_width: 1280.0,
        viewport_css_height: 720.0,
        stream_generation: 3,
    });
    assert_eq!(input.pointer_delta, Vec2::ZERO);

    input.apply_pointer_motion(PointerMotion {
        sequence: 2,
        x_css_pixels: 48.0,
        y_css_pixels: 36.0,
        dx_css_pixels: 12.0,
        dy_css_pixels: 8.0,
        wheel_x: 0.0,
        wheel_y: 0.0,
        viewport_css_width: 1280.0,
        viewport_css_height: 720.0,
        stream_generation: 4,
    });
    assert_eq!(input.pointer_delta, Vec2::new(12.0, 8.0));
}

#[test]
fn remote_motion_uses_authoritative_viewport_cursor_without_publishing_state() {
    let mut input = ViewportNavigationInput::with_viewport_size(100, 80);
    input.apply_pointer_motion(PointerMotion {
        sequence: 1,
        x_css_pixels: 17.0,
        y_css_pixels: 63.0,
        dx_css_pixels: 10.0,
        dy_css_pixels: -5.0,
        wheel_x: 0.0,
        wheel_y: 0.0,
        viewport_css_width: 100.0,
        viewport_css_height: 80.0,
        stream_generation: 1,
    });
    assert_eq!(input.pointer_position, Some(Vec2::new(17.0, 63.0)));
}
