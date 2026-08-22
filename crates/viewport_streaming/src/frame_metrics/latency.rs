use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub(super) struct LatencyAccumulator {
    count: AtomicU64,
    total_ns: AtomicU64,
    max_ns: AtomicU64,
}

impl LatencyAccumulator {
    pub(super) fn record(&self, latency_ns: u64) {
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

    pub(super) fn reset(&self) {
        self.count.store(0, Ordering::Relaxed);
        self.total_ns.store(0, Ordering::Relaxed);
        self.max_ns.store(0, Ordering::Relaxed);
    }

    pub(super) fn snapshot(&self) -> (u64, u64, u64) {
        (
            self.count.load(Ordering::Relaxed),
            self.total_ns.load(Ordering::Relaxed),
            self.max_ns.load(Ordering::Relaxed),
        )
    }
}

pub(super) fn average_ms(count: u64, total_ns: u64) -> Option<f64> {
    (count > 0).then(|| total_ns as f64 / count as f64 / 1_000_000.0)
}

pub(super) fn nanos_to_ms(value: u64) -> Option<f64> {
    (value > 0).then(|| value as f64 / 1_000_000.0)
}
