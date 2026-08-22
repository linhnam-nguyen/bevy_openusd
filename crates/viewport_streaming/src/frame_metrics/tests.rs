use super::*;
use std::thread;
use std::time::Duration;

#[test]
fn traces_are_monotonic_and_timestamped_from_one_clock() {
    let metrics = FrameTransportMetrics::default();
    let first = metrics.next_trace();
    thread::sleep(Duration::from_micros(1));
    let second = metrics.next_trace();

    assert_eq!(first.sequence, 1);
    assert_eq!(second.sequence, 2);
    assert!(second.timestamp_ns >= first.timestamp_ns);
    assert_eq!(first.readback_timestamp_ns, None);
}

#[test]
fn reset_preserves_sequence_but_clears_measurement_counters() {
    let metrics = FrameTransportMetrics::default();
    let trace = metrics.next_trace();
    metrics.record_captured(16, false);
    metrics.record_queued(trace);
    metrics.record_encoder_pushed(trace);
    metrics.record_readback_correlation_overflow();
    metrics.record_readback_correlation_high_water(3);
    metrics.reset();

    let next = metrics.next_trace();
    let snapshot = metrics.snapshot();
    assert_eq!(next.sequence, 2);
    assert_eq!(snapshot.captured_frames, 0);
    assert_eq!(snapshot.queued_frames, 0);
    assert_eq!(snapshot.encoder_pushed, 0);
    assert_eq!(snapshot.readback_correlation_overflows, 0);
    assert_eq!(snapshot.readback_correlation_high_water, 0);
}

#[test]
fn capture_queue_and_generation_drops_are_reported_separately() {
    let metrics = FrameTransportMetrics::default();
    let trace = metrics.next_trace();
    metrics.record_readback_completion();
    metrics.record_captured(128, true);
    metrics.record_queued(trace);
    metrics.record_queue_full_drop();
    metrics.record_generation_drop();
    metrics.record_encoder_queue_drop();
    metrics.record_disconnected_drop();

    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.readback_completions, 1);
    assert_eq!(snapshot.captured_frames, 1);
    assert_eq!(snapshot.queued_frames, 1);
    assert_eq!(snapshot.queue_full_drops, 1);
    assert_eq!(snapshot.readback_repacked_frames, 1);
    assert_eq!(snapshot.generation_drops, 1);
    assert_eq!(snapshot.encoder_queue_drops, 1);
    assert_eq!(snapshot.disconnected_drops, 1);
    assert_eq!(snapshot.readback_copy_bytes, 128);
}

#[test]
fn snapshot_reports_measured_fps_and_stage_semantics() {
    let metrics = FrameTransportMetrics::default();
    metrics.reset();
    let trace = metrics.mark_readback_complete(metrics.next_render_trace());
    metrics.record_queued(trace);
    metrics.record_encoder_queued(trace);
    metrics.record_encoder_worker_started(trace);
    metrics.record_encoder_pushed(trace);

    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.encoder_submitted, 1);
    assert_eq!(snapshot.encoder_pushed, 1);
    assert!(snapshot.measurement_elapsed_ms.is_some());
    assert!(snapshot.encoder_push_fps.is_some());
    assert!(snapshot.readback_to_encoder_queue_avg_ms.is_some());
    assert!(snapshot.readback_to_encoder_worker_avg_ms.is_some());
    assert!(snapshot.readback_to_encoder_push_avg_ms.is_some());
}
