use super::*;

#[test]
fn reset_preserves_configuration_flags() {
    let mut counters = RendererCounters {
        configuration_grid_enabled: false,
        grid_structural_rebuilds: 42,
        semantic_snapshot_clones: 15,
        ..Default::default()
    };

    counters.reset();

    assert_eq!(counters.grid_structural_rebuilds, 0);
    assert_eq!(counters.semantic_snapshot_clones, 0);
    assert!(!counters.configuration_grid_enabled);
}

#[test]
fn query_latency_percentiles_are_derived_from_completed_samples() {
    let mut counters = RendererCounters::default();
    counters.record_query_latency_ms(9.0);
    counters.record_query_latency_ms(1.0);
    counters.record_query_latency_ms(5.0);

    counters.finalize_query_latency();

    assert_eq!(counters.query_median_latency_ms, Some(5.0));
    assert_eq!(counters.query_p95_latency_ms, Some(8.6));
}
