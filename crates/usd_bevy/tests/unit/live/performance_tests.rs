use super::PerformanceCounters;

#[test]
fn disabled_counters_are_no_op() {
    let mut counters = PerformanceCounters::default();
    counters.stage_time_changes(3);
    assert_eq!(counters.stage_time_changes, 0);
}

#[test]
fn reset_preserves_enabled_state() {
    let mut counters = PerformanceCounters {
        enabled: true,
        ..PerformanceCounters::default()
    };
    counters.stage_time_changes(4);
    counters.reset();
    assert!(counters.enabled);
    assert_eq!(counters.stage_time_changes, 0);
}
