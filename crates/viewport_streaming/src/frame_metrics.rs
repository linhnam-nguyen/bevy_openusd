use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Monotonic identity and render/readback timestamps carried with one raw frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameTrace {
    pub sequence: u64,
    /// Timestamp assigned before the frame enters the Bevy render schedule.
    pub timestamp_ns: u64,
    /// Timestamp assigned when the corresponding GPU readback completes.
    pub readback_timestamp_ns: Option<u64>,
}

#[derive(Debug, Default)]
struct LatencyAccumulator {
    count: AtomicU64,
    total_ns: AtomicU64,
    max_ns: AtomicU64,
}

impl LatencyAccumulator {
    fn record(&self, latency_ns: u64) {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.total_ns.fetch_add(latency_ns, Ordering::Relaxed);
        let mut current = self.max_ns.load(Ordering::Relaxed);
        while latency_ns > current {
            match self.max_ns.compare_exchange_weak(
                current,
                latency_ns,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(observed) => current = observed,
            }
        }
    }

    fn reset(&self) {
        self.count.store(0, Ordering::Relaxed);
        self.total_ns.store(0, Ordering::Relaxed);
        self.max_ns.store(0, Ordering::Relaxed);
    }

    fn snapshot(&self) -> (u64, u64, u64) {
        (
            self.count.load(Ordering::Relaxed),
            self.total_ns.load(Ordering::Relaxed),
            self.max_ns.load(Ordering::Relaxed),
        )
    }
}

#[derive(Debug)]
struct FrameTransportMetricsInner {
    started_at: Instant,
    next_sequence: AtomicU64,
    last_queued_sequence: AtomicU64,
    last_encoded_sequence: AtomicU64,
    readback_completions: AtomicU64,
    captured_frames: AtomicU64,
    queued_frames: AtomicU64,
    queue_full_drops: AtomicU64,
    invalid_readbacks: AtomicU64,
    readback_identity_misses: AtomicU64,
    readback_copy_bytes: AtomicU64,
    readback_repacked_frames: AtomicU64,
    generation_drops: AtomicU64,
    encoder_submitted: AtomicU64,
    encoder_queue_drops: AtomicU64,
    encoder_pushed: AtomicU64,
    encoder_failures: AtomicU64,
    disconnected_drops: AtomicU64,
    capture_to_queue: LatencyAccumulator,
    capture_to_encoder: LatencyAccumulator,
}

/// Cross-thread metrics for the render/readback/encode data plane.
///
/// The counters are independent from Bevy's frame counters so a slow encoder
/// cannot block or mutate renderer state. The object is cheap to clone and is
/// intended to be shared by the readback observer, frame pump, and sessions.
#[derive(Clone, Debug)]
pub struct FrameTransportMetrics {
    inner: Arc<FrameTransportMetricsInner>,
}

impl Default for FrameTransportMetrics {
    fn default() -> Self {
        Self {
            inner: Arc::new(FrameTransportMetricsInner {
                started_at: Instant::now(),
                next_sequence: AtomicU64::new(0),
                last_queued_sequence: AtomicU64::new(0),
                last_encoded_sequence: AtomicU64::new(0),
                readback_completions: AtomicU64::new(0),
                captured_frames: AtomicU64::new(0),
                queued_frames: AtomicU64::new(0),
                queue_full_drops: AtomicU64::new(0),
                invalid_readbacks: AtomicU64::new(0),
                readback_identity_misses: AtomicU64::new(0),
                readback_copy_bytes: AtomicU64::new(0),
                readback_repacked_frames: AtomicU64::new(0),
                generation_drops: AtomicU64::new(0),
                encoder_submitted: AtomicU64::new(0),
                encoder_queue_drops: AtomicU64::new(0),
                encoder_pushed: AtomicU64::new(0),
                encoder_failures: AtomicU64::new(0),
                disconnected_drops: AtomicU64::new(0),
                capture_to_queue: LatencyAccumulator::default(),
                capture_to_encoder: LatencyAccumulator::default(),
            }),
        }
    }
}

impl FrameTransportMetrics {
    /// Allocates identity at the render boundary, before GPU work begins.
    pub fn next_render_trace(&self) -> FrameTrace {
        FrameTrace {
            sequence: self.inner.next_sequence.fetch_add(1, Ordering::Relaxed) + 1,
            timestamp_ns: self.timestamp_ns(),
            readback_timestamp_ns: None,
        }
    }

    /// Compatibility alias for callers that do not own the render boundary.
    /// Production readback uses [`Self::next_render_trace`] through the
    /// `FrameReadbackCorrelation` resource instead.
    pub fn next_trace(&self) -> FrameTrace {
        self.next_render_trace()
    }

    /// Attaches the completion timestamp without allocating a new pixel buffer.
    pub fn mark_readback_complete(&self, trace: FrameTrace) -> FrameTrace {
        FrameTrace {
            readback_timestamp_ns: Some(self.timestamp_ns()),
            ..trace
        }
    }

    pub fn timestamp_ns(&self) -> u64 {
        self.inner
            .started_at
            .elapsed()
            .as_nanos()
            .try_into()
            .unwrap_or(u64::MAX)
    }

    pub fn record_readback_completion(&self) {
        self.inner
            .readback_completions
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_invalid_readback(&self) {
        self.inner.invalid_readbacks.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_readback_identity_miss(&self) {
        self.inner
            .readback_identity_misses
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_captured(&self, bytes: usize, repacked: bool) {
        self.inner.captured_frames.fetch_add(1, Ordering::Relaxed);
        self.inner
            .readback_copy_bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
        if repacked {
            self.inner
                .readback_repacked_frames
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_queued(&self, trace: FrameTrace) {
        self.inner.queued_frames.fetch_add(1, Ordering::Relaxed);
        self.inner
            .last_queued_sequence
            .store(trace.sequence, Ordering::Relaxed);
        self.inner
            .capture_to_queue
            .record(self.timestamp_ns().saturating_sub(trace.timestamp_ns));
    }

    pub fn record_queue_full_drop(&self) {
        self.inner.queue_full_drops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_generation_drop(&self) {
        self.inner.generation_drops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_encoder_submitted(&self, trace: FrameTrace) {
        self.inner.encoder_submitted.fetch_add(1, Ordering::Relaxed);
        self.inner
            .capture_to_encoder
            .record(self.timestamp_ns().saturating_sub(trace.timestamp_ns));
    }

    pub fn record_encoder_queue_drop(&self) {
        self.inner
            .encoder_queue_drops
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_encoder_pushed(&self, trace: FrameTrace) {
        self.inner.encoder_pushed.fetch_add(1, Ordering::Relaxed);
        self.inner
            .last_encoded_sequence
            .store(trace.sequence, Ordering::Relaxed);
    }

    pub fn record_encoder_failure(&self) {
        self.inner.encoder_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_disconnected_drop(&self) {
        self.inner
            .disconnected_drops
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Resets the measured window without resetting the process-wide sequence.
    pub fn reset(&self) {
        for counter in [
            &self.inner.last_queued_sequence,
            &self.inner.last_encoded_sequence,
            &self.inner.readback_completions,
            &self.inner.captured_frames,
            &self.inner.queued_frames,
            &self.inner.queue_full_drops,
            &self.inner.invalid_readbacks,
            &self.inner.readback_identity_misses,
            &self.inner.readback_copy_bytes,
            &self.inner.readback_repacked_frames,
            &self.inner.generation_drops,
            &self.inner.encoder_submitted,
            &self.inner.encoder_queue_drops,
            &self.inner.encoder_pushed,
            &self.inner.encoder_failures,
            &self.inner.disconnected_drops,
        ] {
            counter.store(0, Ordering::Relaxed);
        }
        self.inner.capture_to_queue.reset();
        self.inner.capture_to_encoder.reset();
    }

    pub fn snapshot(&self) -> FrameTransportSnapshot {
        let (queue_count, queue_total, queue_max) = self.inner.capture_to_queue.snapshot();
        let (encode_count, encode_total, encode_max) = self.inner.capture_to_encoder.snapshot();
        FrameTransportSnapshot {
            last_queued_sequence: self.inner.last_queued_sequence.load(Ordering::Relaxed),
            last_encoded_sequence: self.inner.last_encoded_sequence.load(Ordering::Relaxed),
            readback_completions: self.inner.readback_completions.load(Ordering::Relaxed),
            captured_frames: self.inner.captured_frames.load(Ordering::Relaxed),
            queued_frames: self.inner.queued_frames.load(Ordering::Relaxed),
            queue_full_drops: self.inner.queue_full_drops.load(Ordering::Relaxed),
            invalid_readbacks: self.inner.invalid_readbacks.load(Ordering::Relaxed),
            readback_identity_misses: self.inner.readback_identity_misses.load(Ordering::Relaxed),
            readback_copy_bytes: self.inner.readback_copy_bytes.load(Ordering::Relaxed),
            readback_repacked_frames: self.inner.readback_repacked_frames.load(Ordering::Relaxed),
            generation_drops: self.inner.generation_drops.load(Ordering::Relaxed),
            encoder_submitted: self.inner.encoder_submitted.load(Ordering::Relaxed),
            encoder_queue_drops: self.inner.encoder_queue_drops.load(Ordering::Relaxed),
            encoder_pushed: self.inner.encoder_pushed.load(Ordering::Relaxed),
            encoder_failures: self.inner.encoder_failures.load(Ordering::Relaxed),
            disconnected_drops: self.inner.disconnected_drops.load(Ordering::Relaxed),
            capture_to_queue_avg_ms: average_ms(queue_count, queue_total),
            capture_to_queue_max_ms: nanos_to_ms(queue_max),
            capture_to_encoder_avg_ms: average_ms(encode_count, encode_total),
            capture_to_encoder_max_ms: nanos_to_ms(encode_max),
        }
    }
}

fn average_ms(count: u64, total_ns: u64) -> Option<f64> {
    (count > 0).then(|| total_ns as f64 / count as f64 / 1_000_000.0)
}

fn nanos_to_ms(value: u64) -> Option<f64> {
    (value > 0).then(|| value as f64 / 1_000_000.0)
}

/// Serializable snapshot used by benchmark reports and checkpoint artifacts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FrameTransportSnapshot {
    pub last_queued_sequence: u64,
    pub last_encoded_sequence: u64,
    pub readback_completions: u64,
    pub captured_frames: u64,
    pub queued_frames: u64,
    pub queue_full_drops: u64,
    pub invalid_readbacks: u64,
    #[serde(default)]
    pub readback_identity_misses: u64,
    pub readback_copy_bytes: u64,
    pub readback_repacked_frames: u64,
    pub generation_drops: u64,
    pub encoder_submitted: u64,
    pub encoder_queue_drops: u64,
    pub encoder_pushed: u64,
    pub encoder_failures: u64,
    pub disconnected_drops: u64,
    pub capture_to_queue_avg_ms: Option<f64>,
    pub capture_to_queue_max_ms: Option<f64>,
    pub capture_to_encoder_avg_ms: Option<f64>,
    pub capture_to_encoder_max_ms: Option<f64>,
}

#[cfg(test)]
mod tests {
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
    }

    #[test]
    fn reset_preserves_sequence_but_clears_measurement_counters() {
        let metrics = FrameTransportMetrics::default();
        let trace = metrics.next_trace();
        metrics.record_captured(16, false);
        metrics.record_queued(trace);
        metrics.record_encoder_pushed(trace);
        metrics.reset();

        let next = metrics.next_trace();
        let snapshot = metrics.snapshot();
        assert_eq!(next.sequence, 2);
        assert_eq!(snapshot.captured_frames, 0);
        assert_eq!(snapshot.queued_frames, 0);
        assert_eq!(snapshot.encoder_pushed, 0);
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
}
