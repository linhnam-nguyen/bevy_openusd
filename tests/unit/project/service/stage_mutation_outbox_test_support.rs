use super::*;

pub(super) fn take_test_failure(queue: &ProjectStageMutationQueue) -> Option<usize> {
    queue
        .test_fail_before_index
        .lock()
        .expect("Project stage mutation queue test hook is not poisoned")
        .take()
}
