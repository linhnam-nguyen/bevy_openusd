use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use super::outbox;
use super::*;

pub(in crate::project) struct OutboxFailureTrigger(Arc<Mutex<Option<usize>>>);

impl OutboxFailureTrigger {
    pub(in crate::project) fn fail_before_batch_index(&self, index: usize) {
        *self.0.lock().expect("stage mutation test fail index lock") = Some(index);
    }
}

struct FailingOutboxSubmitter(Arc<Mutex<Option<usize>>>);

impl OutboxSubmitter for FailingOutboxSubmitter {
    fn submit_batch(
        &self,
        project_root: &Path,
        mutations: &[ProjectStageMutation],
    ) -> Result<(), ProjectWriteError> {
        let fail_before = self
            .0
            .lock()
            .expect("Project stage mutation queue test hook is not poisoned")
            .take();
        outbox::submit_batch_locked_with(project_root, mutations, |index| {
            if fail_before == Some(index) {
                return Err(outbox::filesystem_error());
            }
            Ok(())
        })
    }
}

pub(in crate::project) fn queue_with_failure_trigger()
-> (ProjectStageMutationQueue, OutboxFailureTrigger) {
    let slot = Arc::new(Mutex::new(None));
    let queue = ProjectStageMutationQueue {
        file_lock: Arc::new(Mutex::new(())),
        submitter: Arc::new(FailingOutboxSubmitter(Arc::clone(&slot))),
    };
    (queue, OutboxFailureTrigger(slot))
}

impl ProjectStageMutationQueue {
    pub(crate) fn pending_len_for_project(&self, project_root: &Path) -> usize {
        outbox::read_pending(&outbox_path(project_root))
            .unwrap()
            .len()
    }
}
