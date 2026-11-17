use super::*;

#[test]
fn aggregation_median_and_p95_calculation() {
    let samples: Vec<FrameSample> = (1..=100)
        .map(|i| FrameSample {
            frame_index: i,
            cpu_duration_ms: i as f64,
            wall_interval_ms: Some(i as f64),
            gpu_duration_ms: None,
        })
        .collect();

    let agg = aggregate_frames(&samples, 10);
    assert_eq!(agg.warmup_frames, 10);
    assert_eq!(agg.measured_frames, 90);
    assert!((agg.median_frame_ms - 55.5).abs() < 0.1);
    assert!((agg.p95_frame_ms - 95.55).abs() < 0.1);
    assert_eq!(agg.min_frame_ms, 11.0);
    assert_eq!(agg.max_frame_ms, 100.0);
    assert!(agg.actual_renderer_fps > 0.0);
}

#[test]
fn empty_sample_behavior_returns_zeroes_without_panic() {
    let agg = aggregate_frames(&[], 0);
    assert_eq!(agg.measured_frames, 0);
    assert_eq!(agg.median_frame_ms, 0.0);
    assert_eq!(agg.avg_fps, 0.0);
}
