use super::*;

#[test]
fn tiny_scene_gets_a_near_plane_below_the_legacy_floor() {
    let config = CameraNavigationConfig::default();
    let near = adaptive_near(0.01, 0.02, &config);

    assert!(near.is_finite());
    assert!(near > 0.0);
    assert!(near < 0.001);
}

#[test]
fn large_scene_gets_a_far_plane_beyond_the_legacy_limit() {
    let config = CameraNavigationConfig::default();
    let near = adaptive_near(100_000.0, 100_000.0, &config);
    let far = adaptive_far(100_000.0, 50_000.0, near, &config);

    assert!(near.is_finite());
    assert!(far.is_finite());
    assert!(far > near);
    assert!(far > 10_000.0);
    assert!(f64::from(far) >= 100_000.0 + 50_000.0);
}

#[test]
fn empty_scene_uses_finite_ordered_clipping_fallbacks() {
    let config = CameraNavigationConfig::default();
    let near = adaptive_near(
        config.empty_scene_distance,
        config.empty_scene_radius * 2.0,
        &config,
    );
    let far = adaptive_far(
        config.empty_scene_distance,
        config.empty_scene_radius,
        near,
        &config,
    );

    assert!(near.is_finite() && near > 0.0);
    assert!(far.is_finite() && far > near);
}
