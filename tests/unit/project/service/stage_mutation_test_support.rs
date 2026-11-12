use std::path::Path;

use super::outbox;
use super::*;

impl ProjectStageMutationQueue {
    pub(crate) fn pending_len_for_project(&self, project_root: &Path) -> usize {
        outbox::read_pending(&outbox_path(project_root))
            .unwrap()
            .len()
    }

    pub(crate) fn fail_before_batch_index(&self, index: usize) {
        *self
            .test_fail_before_index
            .lock()
            .expect("stage mutation test fail index lock") = Some(index);
    }
}
