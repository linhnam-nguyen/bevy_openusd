use std::collections::VecDeque;

use bevy::prelude::Resource;
use viewport_protocol::{
    FocusMode, HierarchyNodeId, HierarchySource, RequestId, SceneAnchor, ViewportCommand,
    ViewportCommandEnvelope, ViewportEventEnvelope,
};

/// Tree-specific commands are applied after the scene-anchor index refreshes.
pub(crate) enum ViewportTreeCommand {
    Focus {
        request_id: RequestId,
        target: SceneAnchor,
        mode: FocusMode,
        selection_revision: u64,
        scene_revision: u64,
        generation: u64,
    },
    SetSubtreeVisibility {
        request_id: RequestId,
        target: SceneAnchor,
        visible: bool,
    },
    SetHierarchyNodeVisibility {
        request_id: RequestId,
        source: HierarchySource,
        node_id: HierarchyNodeId,
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
                    | ViewportCommand::SetBimClassificationRecipe { .. }
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
    pending_focus: Option<ViewportTreeCommand>,
    next_focus_generation: u64,
}

impl ViewportTreeCommandInbox {
    pub(crate) fn push(&mut self, command: ViewportTreeCommand) {
        match command {
            ViewportTreeCommand::Focus {
                request_id,
                target,
                mode,
                selection_revision,
                scene_revision,
                ..
            } => {
                self.next_focus_generation = self.next_focus_generation.saturating_add(1);
                self.pending_focus = Some(ViewportTreeCommand::Focus {
                    request_id,
                    target,
                    mode,
                    selection_revision,
                    scene_revision,
                    generation: self.next_focus_generation,
                });
            }
            command => self.pending.push_back(command),
        }
    }

    pub(crate) fn defer_focus(&mut self, command: ViewportTreeCommand) {
        debug_assert!(matches!(command, ViewportTreeCommand::Focus { .. }));
        self.pending_focus = Some(command);
    }

    pub(crate) fn pop(&mut self) -> Option<ViewportTreeCommand> {
        self.pending
            .pop_front()
            .or_else(|| self.pending_focus.take())
    }

    /// Drops focus work captured against an older selection or scene.
    pub(crate) fn cancel_focus_if_stale(&mut self, selection_revision: u64, scene_revision: u64) {
        let stale = self.pending_focus.as_ref().is_some_and(|command| {
            let ViewportTreeCommand::Focus {
                selection_revision: expected_selection,
                scene_revision: expected_scene,
                ..
            } = command
            else {
                return false;
            };
            *expected_selection != selection_revision || *expected_scene != scene_revision
        });
        if stale {
            self.pending_focus = None;
        }
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
#[path = "../../../tests/unit/viewport/api/queues_inline_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../tests/unit/viewport/api/queues_test_support.rs"]
mod test_support;
