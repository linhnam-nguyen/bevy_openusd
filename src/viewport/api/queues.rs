use std::collections::VecDeque;

use bevy::prelude::Resource;
use viewport_protocol::{
    FocusMode, RequestId, SceneAnchor, ViewportCommand, ViewportCommandEnvelope,
    ViewportEventEnvelope,
};

/// Tree-specific commands are applied after the scene-anchor index refreshes.
pub(crate) enum ViewportTreeCommand {
    Focus {
        request_id: RequestId,
        target: SceneAnchor,
        mode: FocusMode,
    },
    SetSubtreeVisibility {
        request_id: RequestId,
        target: SceneAnchor,
        visible: bool,
    },
}

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
    ///
    /// Serialized transports must preserve their host-side correlation IDs;
    /// only in-process callers should use [`Self::send`] to mint `local-*`
    /// IDs.
    pub(crate) fn push(&mut self, envelope: ViewportCommandEnvelope) {
        self.pending.push_back(envelope);
    }

    pub(crate) fn pop(&mut self) -> Option<ViewportCommandEnvelope> {
        self.pending.pop_front()
    }

    /// Removes lazy scene-query commands while preserving the order of all
    /// other viewport commands for the main command system.
    pub(crate) fn take_scene_query_commands(&mut self) -> Vec<ViewportCommandEnvelope> {
        let mut queries = Vec::new();
        let mut remaining = VecDeque::with_capacity(self.pending.len());
        while let Some(envelope) = self.pending.pop_front() {
            if matches!(
                envelope.command,
                ViewportCommand::RequestSceneChildren { .. }
                    | ViewportCommand::SearchScene { .. }
                    | ViewportCommand::SearchBim { .. }
                    | ViewportCommand::RequestBimProperties
                    | ViewportCommand::RequestBimPropertyProvenance { .. }
                    | ViewportCommand::RequestHierarchyChildren { .. }
                    | ViewportCommand::SearchHierarchy { .. }
                    | ViewportCommand::SetHierarchySource { .. }
            ) {
                queries.push(envelope);
            } else {
                remaining.push_back(envelope);
            }
        }
        self.pending = remaining;
        queries
    }
}

#[derive(Resource, Default)]
pub(crate) struct ViewportTreeCommandInbox {
    pending: VecDeque<ViewportTreeCommand>,
}

impl ViewportTreeCommandInbox {
    pub(crate) fn push(&mut self, command: ViewportTreeCommand) {
        self.pending.push_back(command);
    }

    pub(crate) fn push_front(&mut self, command: ViewportTreeCommand) {
        self.pending.push_front(command);
    }

    pub(crate) fn pop(&mut self) -> Option<ViewportTreeCommand> {
        self.pending.pop_front()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_requests_are_ordered_and_identifiable() {
        let mut inbox = ViewportCommandInbox::default();

        assert_eq!(inbox.send(ViewportCommand::ReloadSession), "local-1");
        assert_eq!(inbox.send(ViewportCommand::RequestSnapshot), "local-2");
        assert_eq!(inbox.pop().unwrap().request_id, "local-1");
        assert_eq!(inbox.pop().unwrap().request_id, "local-2");
    }

    #[test]
    fn external_requests_preserve_their_correlation_id() {
        let mut inbox = ViewportCommandInbox::default();
        inbox.push(ViewportCommandEnvelope::new(
            "desktop-3",
            ViewportCommand::RequestSnapshot,
        ));

        assert_eq!(inbox.pop().unwrap().request_id, "desktop-3");
    }

    #[test]
    fn published_events_are_available_to_the_local_reducer_and_transport() {
        let mut outbox = ViewportEventOutbox::default();
        let event = ViewportEventEnvelope::new(
            Some("local-1".into()),
            viewport_protocol::ViewportEvent::Ready {
                protocol_version: viewport_protocol::PROTOCOL_VERSION,
            },
        );

        outbox.push(event.clone());

        assert_eq!(outbox.take_published(), vec![event.clone()]);
        assert_eq!(outbox.pop(), Some(event));
    }
}
