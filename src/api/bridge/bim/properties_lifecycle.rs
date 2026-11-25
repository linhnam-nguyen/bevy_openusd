use bevy::prelude::*;

use super::properties;
use crate::api::ViewportEventOutbox;
use crate::viewport::rendering::scene::SelectedTargets;
use crate::semantic::viewport::{SemanticDiffState, SemanticSyncState};

/// One property request retained until the current generation is readable.
/// The latest request replaces an older one because the frontend read model is
/// request-scoped and only the newest readiness result is authoritative.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub(in crate::api::bridge) struct PendingBimProperties {
    request_id: Option<String>,
    generation: u64,
}

impl PendingBimProperties {
    pub(in crate::api::bridge) fn replace(
        &mut self,
        request_id: String,
        generation: u64,
    ) {
        self.request_id = Some(request_id);
        self.generation = generation;
    }

    pub(in crate::api::bridge) fn take_for(&mut self, generation: u64) -> Option<String> {
        if self.generation != generation {
            return None;
        }
        self.request_id.take()
    }

    pub(in crate::api::bridge) fn discard_stale(&mut self, generation: u64) {
        if self.generation < generation {
            self.request_id = None;
        }
    }
}

pub(in crate::api::bridge) fn replay_pending_bim_properties(
    mut pending: ResMut<PendingBimProperties>,
    stage_info: Res<crate::viewport::stage::StageInfo>,
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
    properties::dispatch(
        request_id,
        Some(&selection),
        Some(&semantic),
        semantic_diff.as_deref(),
        &mut outbox,
    );
}

#[cfg(test)]
#[path = "../../../../tests/unit/viewport/api/bridge/bim/properties_lifecycle_test_support.rs"]
mod test_support;
