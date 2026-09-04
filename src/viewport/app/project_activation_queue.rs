//! Latest-wins admission and cancellation for Project stage preparation.

use std::{
    collections::HashMap,
    sync::{Condvar, Mutex},
    time::Instant,
};

use viewport_streaming::ProjectActivationRequest;

pub(super) struct QueuedProjectActivation {
    pub(super) request: ProjectActivationRequest,
    pub(super) enqueued_at: Instant,
}

pub(super) struct LatestActivationQueue {
    request: Mutex<Option<QueuedProjectActivation>>,
    wake: Condvar,
    closed: Mutex<bool>,
}

impl LatestActivationQueue {
    pub(super) fn new() -> Self {
        Self {
            request: Mutex::new(None),
            wake: Condvar::new(),
            closed: Mutex::new(false),
        }
    }

    pub(super) fn replace(
        &self,
        request: ProjectActivationRequest,
    ) -> Option<ProjectActivationRequest> {
        let mut pending = self
            .request
            .lock()
            .expect("Project activation request queue is not poisoned");
        let replaced = pending.replace(QueuedProjectActivation {
            request,
            enqueued_at: Instant::now(),
        });
        self.wake.notify_one();
        replaced.map(|queued| queued.request)
    }

    pub(super) fn take(&self) -> Option<QueuedProjectActivation> {
        let mut pending = self
            .request
            .lock()
            .expect("Project activation request queue is not poisoned");
        loop {
            if let Some(request) = pending.take() {
                return Some(request);
            }
            if *self
                .closed
                .lock()
                .expect("Project activation queue state is not poisoned")
            {
                return None;
            }
            pending = self
                .wake
                .wait(pending)
                .expect("Project activation request queue is not poisoned");
        }
    }

    pub(super) fn close(&self) {
        *self
            .closed
            .lock()
            .expect("Project activation queue state is not poisoned") = true;
        self.wake.notify_one();
    }
}

#[derive(Default)]
pub(super) struct ActivationCancellation {
    latest_by_session: Mutex<HashMap<String, String>>,
}

impl ActivationCancellation {
    pub(super) fn supersede(&self, request: &ProjectActivationRequest) {
        self.latest_by_session
            .lock()
            .expect("Project activation cancellation state is not poisoned")
            .insert(
                request.session_id.0.clone(),
                request.command.request_id.clone(),
            );
    }

    pub(super) fn is_current(&self, request: &ProjectActivationRequest) -> bool {
        self.latest_by_session
            .lock()
            .expect("Project activation cancellation state is not poisoned")
            .get(&request.session_id.0)
            .is_some_and(|request_id| request_id == &request.command.request_id)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::*;
    use project_protocol::{ProjectActivationCommand, ProjectStageTarget};
    use usd_project::{ProjectId, ProjectRoot};
    use viewport_protocol::SessionId;

    fn request(request_id: &str) -> ProjectActivationRequest {
        ProjectActivationRequest {
            session_id: SessionId::new("session-a"),
            command: ProjectActivationCommand::new(
                request_id,
                1,
                ProjectId::new_v4(),
                ProjectStageTarget::ProjectRoot(ProjectRoot::Empty),
            ),
        }
    }

    #[test]
    fn latest_request_replaces_blocked_preparation() {
        let queue = LatestActivationQueue::new();
        assert!(queue.replace(request("A")).is_none());
        assert_eq!(
            queue
                .replace(request("B"))
                .expect("A should be superseded")
                .command
                .request_id,
            "A"
        );
        assert_eq!(
            queue
                .take()
                .expect("B should remain queued")
                .request
                .command
                .request_id,
            "B"
        );
    }

    #[test]
    fn superseded_request_is_not_current() {
        let cancellation = ActivationCancellation::default();
        let first = request("A");
        let second = request("B");
        cancellation.supersede(&first);
        cancellation.supersede(&second);

        assert!(!cancellation.is_current(&first));
        assert!(cancellation.is_current(&second));
    }

    #[test]
    fn blocked_preparation_accepts_newest_request_without_waiting_for_old_one() {
        let queue = Arc::new(LatestActivationQueue::new());
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let worker_queue = Arc::clone(&queue);
        let worker_entered = Arc::clone(&entered);
        let worker_release = Arc::clone(&release);
        let worker = std::thread::spawn(move || {
            let first = worker_queue.take().expect("A should be admitted");
            worker_entered.wait();
            worker_release.wait();
            let second = worker_queue.take().expect("B should remain available");
            (
                first.request.command.request_id,
                second.request.command.request_id,
            )
        });

        assert!(queue.replace(request("A")).is_none());
        entered.wait();
        assert!(
            queue.replace(request("B")).is_none(),
            "B admission must not wait for in-flight A"
        );
        release.wait();

        assert_eq!(
            worker.join().expect("blocked worker should finish"),
            ("A".to_owned(), "B".to_owned())
        );
    }
}
