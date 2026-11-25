use std::collections::VecDeque;

use bevy::prelude::Resource;
use viewport_protocol::{FocusMode, HierarchyNodeId, HierarchySource, RequestId, SceneAnchor};

pub(crate) use crate::viewport::presentation::{ViewportCommandInbox, ViewportEventOutbox};

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

#[cfg(test)]
#[path = "../../tests/unit/viewport/api/queues_inline_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/viewport/api/queues_test_support.rs"]
mod test_support;
