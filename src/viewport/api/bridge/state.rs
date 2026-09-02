use std::collections::HashMap;
use std::time::Instant;

use bevy::prelude::*;
use usd_bevy::LiveStage;
use viewport_protocol::{EditorStateReadModel, RuntimeMutationBatch};

/// Tracks undo/redo stacks for authoring and transform operations.
#[derive(Resource, Default)]
pub(super) struct EditorHistories {
    pub authoring: usd_bevy::authoring::EditHistory,
    pub transforms: usd_bevy::TransformHistory,
    pub undo_domains: Vec<EditorHistoryDomain>,
    pub redo_domains: Vec<EditorHistoryDomain>,
    pub is_dirty: bool,
}

/// Main-thread admission control for connector-originated writes.
///
/// OpenUSD stages are non-send and the normal viewport command system is the
/// single writer. This resource adds source sequence and live-revision checks
/// before a runtime batch reaches the existing authoring APIs.
#[derive(Resource, Default)]
pub(super) struct RuntimeMutationCoordinator {
    pub last_sequence_by_source: HashMap<String, u64>,
}

#[derive(Resource, Default)]
pub(super) struct SceneSearchRequests {
    pub pending: HashMap<String, SceneSearchRequest>,
}

pub(super) struct SceneSearchRequest {
    pub query: String,
    pub offset: u32,
    pub submitted_at: Instant,
}

#[derive(Clone, Copy)]
pub(super) enum EditorHistoryDomain {
    Authoring,
    Transform,
}

/// Explicit ordering points around the public viewport bridge.
///
/// Native transports enqueue work before [`Self::ApplyCommands`] and drain
/// resulting events after [`Self::PublishStageLoadState`]. This preserves the
/// existing Frost behavior while avoiding a blocking pipe operation inside an
/// ECS command system.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ViewportBridgeSet {
    RefreshSceneIndex,
    ApplyCommands,
    ApplyTreeCommands,
    PublishStageLoadState,
    ReduceEvents,
}

impl RuntimeMutationCoordinator {
    pub fn admit(&self, stage: &LiveStage, batch: &RuntimeMutationBatch) -> Result<(), String> {
        if stage.has_changes() {
            return Err(
                "live stage has a pending change batch; retry after the next authoritative revision"
                    .to_owned(),
            );
        }
        if batch.base_revision != stage.current_revision().0 {
            return Err(format!(
                "stale runtime base revision {}; current revision is {}",
                batch.base_revision,
                stage.current_revision().0
            ));
        }
        if self
            .last_sequence_by_source
            .get(&batch.source_id)
            .is_some_and(|last| batch.sequence <= *last)
        {
            return Err(format!(
                "runtime sequence {} for source {} is not newer than the last accepted sequence",
                batch.sequence, batch.source_id
            ));
        }
        Ok(())
    }

    pub fn record(&mut self, batch: &RuntimeMutationBatch) {
        self.last_sequence_by_source
            .insert(batch.source_id.clone(), batch.sequence);
    }

    pub fn reset(&mut self) {
        self.last_sequence_by_source.clear();
    }
}

impl EditorHistories {
    pub fn record(&mut self, domain: EditorHistoryDomain) {
        self.undo_domains.push(domain);
        self.redo_domains.clear();
        self.is_dirty = true;
    }

    pub fn mark_dirty(&mut self) {
        self.is_dirty = true;
    }

    pub fn mark_saved(&mut self) {
        self.is_dirty = false;
    }

    pub fn state(&self) -> EditorStateReadModel {
        EditorStateReadModel {
            can_undo: !self.undo_domains.is_empty(),
            can_redo: !self.redo_domains.is_empty(),
            is_dirty: self.is_dirty,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_state_is_separate_from_undo_history() {
        let mut histories = EditorHistories::default();
        histories.record(EditorHistoryDomain::Authoring);
        assert!(histories.state().is_dirty);
        assert!(histories.state().can_undo);

        histories.mark_saved();

        assert!(!histories.state().is_dirty);
        assert!(histories.state().can_undo);
    }
}
