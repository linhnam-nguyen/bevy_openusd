use super::*;

#[test]
fn server_normalization_clamps_dimensions_and_fps() {
    let metrics = ViewportMetrics {
        css_width: 8_000,
        css_height: 4_000,
        device_pixel_ratio: 2.0,
        requested_width: 16_000,
        requested_height: 8_000,
        preferred_fps: Some(240),
        generation: 9,
    };

    let normalized = StreamLimits::default().normalize(&metrics);

    assert_eq!(normalized.requested_width, 5_460);
    assert_eq!(normalized.requested_height, 3_072);
    assert_eq!(normalized.preferred_fps, Some(240));
    assert_eq!(normalized.generation, 9);
}

#[test]
fn normalized_dimensions_are_even_and_within_pixel_budget() {
    let metrics = ViewportMetrics {
        requested_width: 7_679,
        requested_height: 4_319,
        ..ViewportMetrics::default()
    };
    let normalized = StreamLimits::default().normalize(&metrics);

    assert_eq!(normalized.requested_width % 2, 0);
    assert_eq!(normalized.requested_height % 2, 0);
    assert!(
        u64::from(normalized.requested_width) * u64::from(normalized.requested_height)
            <= MAX_PIXEL_COUNT
    );
}
