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
#[path = "../../../tests/unit/viewport/app/project_activation_queue_inline_tests.rs"]
mod tests;
