use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub(super) struct FrameSignatureMetrics {
    last_hash: AtomicU64,
    frame_count: AtomicU64,
    last_mad_luma_bits: AtomicU64,
    mad_frame_count: AtomicU64,
}

impl FrameSignatureMetrics {
    pub(super) fn record(&self, hash: u64, mad_luma: Option<f64>) {
        self.last_hash.store(hash, Ordering::Relaxed);
        self.frame_count.fetch_add(1, Ordering::Release);
        if let Some(mad_luma) = mad_luma.filter(|value| value.is_finite() && *value >= 0.0) {
            self.last_mad_luma_bits
                .store(mad_luma.to_bits(), Ordering::Relaxed);
            self.mad_frame_count.fetch_add(1, Ordering::Release);
        }
    }

    pub(super) fn reset(&self) {
        self.last_hash.store(0, Ordering::Relaxed);
        self.frame_count.store(0, Ordering::Relaxed);
        self.last_mad_luma_bits.store(0, Ordering::Relaxed);
        self.mad_frame_count.store(0, Ordering::Relaxed);
    }

    pub(super) fn snapshot(&self) -> (Option<u64>, u64, u64, Option<f64>) {
        let frame_count = self.frame_count.load(Ordering::Acquire);
        let mad_frame_count = self.mad_frame_count.load(Ordering::Acquire);
        (
            (frame_count > 0).then(|| self.last_hash.load(Ordering::Relaxed)),
            frame_count,
            mad_frame_count,
            (mad_frame_count > 0)
                .then(|| f64::from_bits(self.last_mad_luma_bits.load(Ordering::Relaxed))),
        )
    }
}
