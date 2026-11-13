use super::*;

#[test]
fn zoom_ratio_is_scale_independent() {
    let config = CameraNavigationConfig::default();
    let small = zoom_target_after_scroll(0.001, -10.0, config.zoom_speed) / 0.001;
    let large = zoom_target_after_scroll(1_000_000.0, -10.0, config.zoom_speed) / 1_000_000.0;

    assert!((small - large).abs() / small < 1.0e-6);
}

#[test]
fn zoom_has_no_legacy_user_scale_limits() {
    let config = CameraNavigationConfig::default();
    let closer = zoom_target_after_scroll(0.001, 10.0, config.zoom_speed);
    let farther = zoom_target_after_scroll(60.0, -120.0, config.zoom_speed);

    assert!(closer < 0.001);
    assert!(farther > 60.0);
}

#[test]
fn repeated_extreme_scroll_stays_finite() {
    let config = CameraNavigationConfig::default();
    let mut distance = 1.0;
    for _ in 0..1_000 {
        distance = zoom_target_after_scroll(distance, -120.0, config.zoom_speed);
        distance = zoom_target_after_scroll(distance, 120.0, config.zoom_speed);
    }

    assert!(distance.is_finite());
    assert!(distance > 0.0);
}
