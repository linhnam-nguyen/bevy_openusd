use bevy::prelude::*;

use super::bim_properties;
use crate::viewport::api::ViewportEventOutbox;
use crate::viewport::scene::SelectedTargets;
use crate::viewport::semantic::{SemanticDiffState, SemanticSyncState};

/// One property request retained until the current generation is readable.
/// The latest request replaces an older one because the frontend read model is
/// request-scoped and only the newest readiness result is authoritative.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct PendingBimProperties {
    request_id: Option<String>,
    generation: u64,
}

impl PendingBimProperties {
    pub(super) fn replace(&mut self, request_id: String, generation: u64) {
        self.request_id = Some(request_id);
        self.generation = generation;
    }

    pub(super) fn take_for(&mut self, generation: u64) -> Option<String> {
        if self.generation != generation {
            return None;
        }
        self.request_id.take()
    }

    pub(super) fn discard_stale(&mut self, generation: u64) {
        if self.generation < generation {
            self.request_id = None;
        }
    }

    #[cfg(test)]
    pub(crate) fn has_request(&self) -> bool {
        self.request_id.is_some()
    }
}

pub(super) fn replay_pending_bim_properties(
    mut pending: ResMut<PendingBimProperties>,
    stage_info: Res<crate::viewport::session::StageInfo>,
    selection: Option<Res<SelectedTargets>>,
    semantic: Option<Res<SemanticSyncState>>,
    semantic_diff: Option<Res<SemanticDiffState>>,
    mut outbox: ResMut<ViewportEventOutbox>,
) {
    pending.discard_stale(stage_info.activation_generation);
    let Some(selection) = selection else {
        return;
    };
    let Some(semantic) = semantic else {
        return;
    };
    if selection.0.targets.is_empty()
        || semantic.snapshot().is_none()
        || semantic.shared_bim_index().is_none()
    {
        return;
    }
    let Some(request_id) = pending.take_for(stage_info.activation_generation) else {
        return;
    };
    bim_properties::dispatch(
        request_id,
        Some(&selection),
        Some(&semantic),
        semantic_diff.as_deref(),
        &mut outbox,
    );
}
