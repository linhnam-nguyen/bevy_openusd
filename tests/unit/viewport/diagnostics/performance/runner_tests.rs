use super::*;

#[test]
fn benchmark_run_state_transitions() {
    let mut state = BenchmarkRunState::new(2, 3);
    assert_eq!(state.warmup_frames_remaining, 2);
    assert_eq!(state.target_frames_remaining, 3);
    assert!(!state.is_completed);

    state.warmup_frames_remaining = 0;
    state.target_frames_remaining = 0;
    state.is_completed = true;

    assert!(state.is_completed);
}
