//! Test-only helpers attached to the owning production module.
use super::*;

impl TargetedRepairPersistenceWorker {
    pub(crate) fn with_completion_for_test(
        completion: TargetedRepairPersistenceCompletion,
    ) -> Self {
        let (completion_tx, completion_rx) = mpsc::sync_channel(QUEUE_CAPACITY);
        completion_tx
            .send(completion)
            .expect("test repair completion queue accepts one completion");
        drop(completion_tx);
        Self {
            requests: None,
            completions: Some(Mutex::new(completion_rx)),
            thread: None,
            available: AtomicBool::new(false),
        }
    }
}
