//! Test-only helpers attached to the owning production module.
use super::*;

impl CachedResidencyWorker {
    pub(crate) fn unavailable_for_test() -> Self {
        Self {
            requests: None,
            completions: None,
            thread: None,
            available: AtomicBool::new(false),
        }
    }

    pub(crate) fn with_completion_for_test(completion: LoadCompletion) -> Self {
        let (completion_tx, completion_rx) = mpsc::sync_channel(WORKER_QUEUE_CAPACITY);
        completion_tx
            .send(completion)
            .expect("test completion queue accepts one completion");
        drop(completion_tx);
        Self {
            requests: None,
            completions: Some(Mutex::new(completion_rx)),
            thread: None,
            available: AtomicBool::new(false),
        }
    }
}
