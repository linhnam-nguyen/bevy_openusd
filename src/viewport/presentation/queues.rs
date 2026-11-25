//! Neutral command/event queues shared between generic viewport systems and
//! external protocol/API adapters.

use std::collections::VecDeque;

use bevy::prelude::Resource;
use viewport_protocol::{
    RequestId, ViewportCommand, ViewportCommandEnvelope, ViewportEventEnvelope,
};

/// Commands accepted from in-process adapters and serialized transports.
#[derive(Resource, Default)]
pub(crate) struct ViewportCommandInbox {
    next_request_id: u64,
    pending: VecDeque<ViewportCommandEnvelope>,
}

impl ViewportCommandInbox {
    /// Queues a command with a monotonically increasing in-process request ID.
    pub(crate) fn send(&mut self, command: ViewportCommand) -> RequestId {
        self.next_request_id = self.next_request_id.saturating_add(1);
        let request_id = format!("local-{}", self.next_request_id);
        self.pending
            .push_back(ViewportCommandEnvelope::new(request_id.clone(), command));
        request_id
    }

    /// Queues a command that already has a caller-assigned request ID.
    pub(crate) fn push(&mut self, envelope: ViewportCommandEnvelope) {
        self.pending.push_back(envelope);
    }

    pub(crate) fn pop(&mut self) -> Option<ViewportCommandEnvelope> {
        self.pending.pop_front()
    }

    pub(crate) fn retain(&mut self, keep: impl FnMut(&ViewportCommandEnvelope) -> bool) {
        self.pending.retain(keep);
    }

    /// Stably drains commands matching `predicate` while preserving the FIFO
    /// order of all remaining commands. Product-specific query routing
    /// predicates remain owned by `crate::api`.
    pub(crate) fn drain_matching(
        &mut self,
        mut predicate: impl FnMut(&ViewportCommandEnvelope) -> bool,
    ) -> Vec<ViewportCommandEnvelope> {
        let mut matched = Vec::new();
        let mut remaining = VecDeque::with_capacity(self.pending.len());
        while let Some(envelope) = self.pending.pop_front() {
            if predicate(&envelope) {
                matched.push(envelope);
            } else {
                remaining.push_back(envelope);
            }
        }
        self.pending = remaining;
        matched
    }
}

/// Events emitted after the native viewport applies a command.
#[derive(Resource, Default)]
pub(crate) struct ViewportEventOutbox {
    pending: VecDeque<ViewportEventEnvelope>,
    published: VecDeque<ViewportEventEnvelope>,
}

impl ViewportEventOutbox {
    pub(crate) fn push(&mut self, event: ViewportEventEnvelope) {
        self.published.push_back(event.clone());
        self.pending.push_back(event);
    }

    pub(crate) fn push_front(&mut self, event: ViewportEventEnvelope) {
        self.pending.push_front(event);
    }

    #[allow(dead_code)]
    pub(crate) fn pop(&mut self) -> Option<ViewportEventEnvelope> {
        self.pending.pop_front()
    }

    /// Drains newly emitted events without consuming the transport queue.
    pub(crate) fn take_published(&mut self) -> Vec<ViewportEventEnvelope> {
        self.published.drain(..).collect()
    }
}
