mod latency;
mod snapshot;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use latency::{LatencyAccumulator, average_ms, nanos_to_ms};
pub use snapshot::FrameTransportSnapshot;

/// Monotonic identity and render/readback timestamps carried with one raw frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameTrace {
    pub sequence: u64,
    /// Timestamp assigned before the frame enters the Bevy render schedule.
    pub timestamp_ns: u64,
    /// Timestamp assigned when the corresponding GPU readback completes.
    pub readback_timestamp_ns: Option<u64>,
}

#[derive(Debug)]
struct FrameTransportMetricsInner {
    started_at: Instant,
    measurement_started_ns: AtomicU64,
    next_sequence: AtomicU64,
    last_queued_sequence: AtomicU64,
    last_encoded_sequence: AtomicU64,
    readback_completions: AtomicU64,
    captured_frames: AtomicU64,
    queued_frames: AtomicU64,
    queue_full_drops: AtomicU64,
    invalid_readbacks: AtomicU64,
    readback_identity_misses: AtomicU64,
    readback_correlation_overflows: AtomicU64,
    readback_correlation_high_water: AtomicU64,
    readback_copy_bytes: AtomicU64,
    readback_repacked_frames: AtomicU64,
    generation_drops: AtomicU64,
    encoder_submitted: AtomicU64,
    encoder_queue_drops: AtomicU64,
    encoder_pushed: AtomicU64,
    encoder_failures: AtomicU64,
    disconnected_drops: AtomicU64,
    render_to_readback: LatencyAccumulator,
    readback_to_queue: LatencyAccumulator,
    readback_to_encoder_queue: LatencyAccumulator,
    readback_to_encoder_worker: LatencyAccumulator,
    readback_to_encoder_push: LatencyAccumulator,
}

/// Cross-thread metrics for the render/readback/encode data plane.
#[derive(Clone, Debug)]
pub struct FrameTransportMetrics {
    inner: Arc<FrameTransportMetricsInner>,
}

impl Default for FrameTransportMetrics {
    fn default() -> Self {
        Self {
            inner: Arc::new(FrameTransportMetricsInner {
                started_at: Instant::now(),
                measurement_started_ns: AtomicU64::new(0),
                next_sequence: AtomicU64::new(0),
                last_queued_sequence: AtomicU64::new(0),
                last_encoded_sequence: AtomicU64::new(0),
                readback_completions: AtomicU64::new(0),
                captured_frames: AtomicU64::new(0),
                queued_frames: AtomicU64::new(0),
                queue_full_drops: AtomicU64::new(0),
                invalid_readbacks: AtomicU64::new(0),
                readback_identity_misses: AtomicU64::new(0),
                readback_correlation_overflows: AtomicU64::new(0),
                readback_correlation_high_water: AtomicU64::new(0),
                readback_copy_bytes: AtomicU64::new(0),
                readback_repacked_frames: AtomicU64::new(0),
                generation_drops: AtomicU64::new(0),
                encoder_submitted: AtomicU64::new(0),
                encoder_queue_drops: AtomicU64::new(0),
                encoder_pushed: AtomicU64::new(0),
                encoder_failures: AtomicU64::new(0),
                disconnected_drops: AtomicU64::new(0),
                render_to_readback: LatencyAccumulator::default(),
                readback_to_queue: LatencyAccumulator::default(),
                readback_to_encoder_queue: LatencyAccumulator::default(),
                readback_to_encoder_worker: LatencyAccumulator::default(),
                readback_to_encoder_push: LatencyAccumulator::default(),
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
    pub fn next_trace(&self) -> FrameTrace {
        self.next_render_trace()
    }

    /// Attaches the completion timestamp without allocating a new pixel buffer.
    pub fn mark_readback_complete(&self, trace: FrameTrace) -> FrameTrace {
        let timestamp_ns = self.timestamp_ns();
        self.record_latency(
            &self.inner.render_to_readback,
            trace.timestamp_ns,
            timestamp_ns,
        );
        FrameTrace {
            readback_timestamp_ns: Some(timestamp_ns),
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

    pub fn record_readback_correlation_overflow(&self) {
        self.inner
            .readback_correlation_overflows
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_readback_correlation_high_water(&self, high_water: usize) {
        let high_water = high_water as u64;
        let mut current = self
            .inner
            .readback_correlation_high_water
            .load(Ordering::Relaxed);
        while high_water > current {
            match self
                .inner
                .readback_correlation_high_water
                .compare_exchange_weak(current, high_water, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => break,
                Err(observed) => current = observed,
            }
        }
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
        if let Some(readback_timestamp_ns) = trace.readback_timestamp_ns {
            self.record_latency(
                &self.inner.readback_to_queue,
                readback_timestamp_ns,
                self.timestamp_ns(),
            );
        }
    }

    pub fn record_queue_full_drop(&self) {
        self.inner.queue_full_drops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_generation_drop(&self) {
        self.inner.generation_drops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_encoder_queued(&self, trace: FrameTrace) {
        self.inner.encoder_submitted.fetch_add(1, Ordering::Relaxed);
        if let Some(readback_timestamp_ns) = trace.readback_timestamp_ns {
            self.record_latency(
                &self.inner.readback_to_encoder_queue,
                readback_timestamp_ns,
                self.timestamp_ns(),
            );
        }
    }

    pub fn record_encoder_worker_started(&self, trace: FrameTrace) {
        if let Some(readback_timestamp_ns) = trace.readback_timestamp_ns {
            self.record_latency(
                &self.inner.readback_to_encoder_worker,
                readback_timestamp_ns,
                self.timestamp_ns(),
            );
        }
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
        if let Some(readback_timestamp_ns) = trace.readback_timestamp_ns {
            self.record_latency(
                &self.inner.readback_to_encoder_push,
                readback_timestamp_ns,
                self.timestamp_ns(),
            );
        }
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
        self.inner
            .measurement_started_ns
            .store(self.timestamp_ns(), Ordering::Relaxed);
        for counter in [
            &self.inner.last_queued_sequence,
            &self.inner.last_encoded_sequence,
            &self.inner.readback_completions,
            &self.inner.captured_frames,
            &self.inner.queued_frames,
            &self.inner.queue_full_drops,
            &self.inner.invalid_readbacks,
            &self.inner.readback_identity_misses,
            &self.inner.readback_correlation_overflows,
            &self.inner.readback_correlation_high_water,
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
        self.inner.render_to_readback.reset();
        self.inner.readback_to_queue.reset();
        self.inner.readback_to_encoder_queue.reset();
        self.inner.readback_to_encoder_worker.reset();
        self.inner.readback_to_encoder_push.reset();
    }

    pub fn snapshot(&self) -> FrameTransportSnapshot {
        let now_ns = self.timestamp_ns();
        let measurement_elapsed_ns =
            now_ns.saturating_sub(self.inner.measurement_started_ns.load(Ordering::Relaxed));
        let (render_readback_count, render_readback_total, render_readback_max) =
            self.inner.render_to_readback.snapshot();
        let (readback_queue_count, readback_queue_total, readback_queue_max) =
            self.inner.readback_to_queue.snapshot();
        let (encoder_queue_count, encoder_queue_total, encoder_queue_max) =
            self.inner.readback_to_encoder_queue.snapshot();
        let (encoder_worker_count, encoder_worker_total, encoder_worker_max) =
            self.inner.readback_to_encoder_worker.snapshot();
        let (encoder_push_count, encoder_push_total, encoder_push_max) =
            self.inner.readback_to_encoder_push.snapshot();
        let readback_completions = self.inner.readback_completions.load(Ordering::Relaxed);
        let encoder_pushed = self.inner.encoder_pushed.load(Ordering::Relaxed);

        FrameTransportSnapshot {
            last_queued_sequence: self.inner.last_queued_sequence.load(Ordering::Relaxed),
            last_encoded_sequence: self.inner.last_encoded_sequence.load(Ordering::Relaxed),
            readback_completions,
            captured_frames: self.inner.captured_frames.load(Ordering::Relaxed),
            queued_frames: self.inner.queued_frames.load(Ordering::Relaxed),
            queue_full_drops: self.inner.queue_full_drops.load(Ordering::Relaxed),
            invalid_readbacks: self.inner.invalid_readbacks.load(Ordering::Relaxed),
            readback_identity_misses: self.inner.readback_identity_misses.load(Ordering::Relaxed),
            readback_correlation_overflows: self
                .inner
                .readback_correlation_overflows
                .load(Ordering::Relaxed),
            readback_correlation_high_water: self
                .inner
                .readback_correlation_high_water
                .load(Ordering::Relaxed),
            readback_copy_bytes: self.inner.readback_copy_bytes.load(Ordering::Relaxed),
            readback_repacked_frames: self.inner.readback_repacked_frames.load(Ordering::Relaxed),
            generation_drops: self.inner.generation_drops.load(Ordering::Relaxed),
            encoder_submitted: self.inner.encoder_submitted.load(Ordering::Relaxed),
            encoder_queue_drops: self.inner.encoder_queue_drops.load(Ordering::Relaxed),
            encoder_pushed: self.inner.encoder_pushed.load(Ordering::Relaxed),
            encoder_failures: self.inner.encoder_failures.load(Ordering::Relaxed),
            disconnected_drops: self.inner.disconnected_drops.load(Ordering::Relaxed),
            measurement_elapsed_ms: nanos_to_ms(measurement_elapsed_ns),
            readback_fps: frames_per_second(readback_completions, measurement_elapsed_ns),
            encoder_push_fps: frames_per_second(encoder_pushed, measurement_elapsed_ns),
            render_to_readback_avg_ms: average_ms(render_readback_count, render_readback_total),
            render_to_readback_max_ms: nanos_to_ms(render_readback_max),
            readback_to_queue_avg_ms: average_ms(readback_queue_count, readback_queue_total),
            readback_to_queue_max_ms: nanos_to_ms(readback_queue_max),
            readback_to_encoder_queue_avg_ms: average_ms(encoder_queue_count, encoder_queue_total),
            readback_to_encoder_queue_max_ms: nanos_to_ms(encoder_queue_max),
            readback_to_encoder_worker_avg_ms: average_ms(
                encoder_worker_count,
                encoder_worker_total,
            ),
            readback_to_encoder_worker_max_ms: nanos_to_ms(encoder_worker_max),
            readback_to_encoder_push_avg_ms: average_ms(encoder_push_count, encoder_push_total),
            readback_to_encoder_push_max_ms: nanos_to_ms(encoder_push_max),
        }
    }

    fn record_latency(&self, accumulator: &LatencyAccumulator, start_ns: u64, end_ns: u64) {
        let measurement_start_ns = self.inner.measurement_started_ns.load(Ordering::Relaxed);
        if start_ns >= measurement_start_ns && end_ns >= start_ns {
            accumulator.record(end_ns - start_ns);
        }
    }
}

fn frames_per_second(frames: u64, elapsed_ns: u64) -> Option<f64> {
    (frames > 0 && elapsed_ns > 0).then(|| frames as f64 / (elapsed_ns as f64 / 1_000_000_000.0))
}
