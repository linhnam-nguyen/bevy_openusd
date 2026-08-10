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

/// Commands accepted from Frost now and from a future serialized transport.
#[derive(Resource, Default)]
pub(crate) struct ViewportCommandInbox {
    next_request_id: u64,
    pending: VecDeque<ViewportCommandEnvelope>,
}

impl ViewportCommandInbox {
    /// Queues a command with a monotonically increasing in-process request ID.
    pub(crate) fn send(&mut self, command: ViewportCommand) -> RequestId {
        self.next_request_id = self.next_request_id.saturating_add(1);
        let request_id = format!("frost-{}", self.next_request_id);
        self.pending
            .push_back(ViewportCommandEnvelope::new(request_id.clone(), command));
        request_id
    }

    /// Queues a command that already has a caller-assigned request ID.
    ///
    /// Serialized transports must preserve their host-side correlation IDs;
    /// only in-process callers should use [`Self::send`] to mint `frost-*`
    /// IDs.
    pub(crate) fn push(&mut self, envelope: ViewportCommandEnvelope) {
        self.pending.push_back(envelope);
    }

    pub(crate) fn pop(&mut self) -> Option<ViewportCommandEnvelope> {
        self.pending.pop_front()
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

    pub(crate) fn pop(&mut self) -> Option<ViewportTreeCommand> {
        self.pending.pop_front()
    }
}

/// Events emitted after the native viewport applies a command.
#[derive(Resource, Default)]
pub(crate) struct ViewportEventOutbox {
    pending: VecDeque<ViewportEventEnvelope>,
}

impl ViewportEventOutbox {
    pub(crate) fn push(&mut self, event: ViewportEventEnvelope) {
        self.pending.push_back(event);
    }

    #[allow(dead_code)]
    pub(crate) fn pop(&mut self) -> Option<ViewportEventEnvelope> {
        self.pending.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_requests_are_ordered_and_identifiable() {
        let mut inbox = ViewportCommandInbox::default();

        assert_eq!(inbox.send(ViewportCommand::ReloadSession), "frost-1");
        assert_eq!(inbox.send(ViewportCommand::RequestSnapshot), "frost-2");
        assert_eq!(inbox.pop().unwrap().request_id, "frost-1");
        assert_eq!(inbox.pop().unwrap().request_id, "frost-2");
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
}
