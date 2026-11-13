use std::sync::Arc;

use bevy::prelude::Resource;
use usd_bevy::LiveRevision;
use usd_model::SemanticSnapshot;
use usd_semantic::SemanticConfig;

use crate::viewport::bim::BimReadIndex;

/// Local authoritative semantic state used to derive the next incremental
/// update from the same live-stage revision consumed by Bevy projection.
#[derive(Default, Resource)]
pub(crate) struct SemanticSyncState {
    config: SemanticConfig,
    pub(super) snapshot: Option<Arc<SemanticSnapshot>>,
    pub(super) bim_index: Option<Arc<BimReadIndex>>,
    pub(super) session_id: Option<u64>,
    pub(super) revision: Option<LiveRevision>,
    pub(super) activation_generation: u64,
}

impl SemanticSyncState {
    pub(crate) fn with_config(config: SemanticConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

    pub(crate) fn config(&self) -> SemanticConfig {
        self.config.clone()
    }

    pub(crate) fn snapshot(&self) -> Option<&SemanticSnapshot> {
        self.snapshot.as_deref()
    }

    pub(crate) fn shared_snapshot(&self) -> Option<Arc<SemanticSnapshot>> {
        self.snapshot.as_ref().map(Arc::clone)
    }

    pub(crate) fn shared_bim_index(&self) -> Option<Arc<BimReadIndex>> {
        self.bim_index.as_ref().map(Arc::clone)
    }

    pub(crate) fn activation_generation(&self) -> u64 {
        self.activation_generation
    }

    /// Invalidates all derived semantic state at a successful stage boundary.
    /// The generation is retained even while the next immutable snapshot is
    /// being extracted, so consumers can reject work from the previous stage.
    pub(crate) fn reset_for_activation(&mut self, activation_generation: u64) {
        self.snapshot = None;
        self.bim_index = None;
        self.session_id = None;
        self.revision = None;
        self.activation_generation = activation_generation;
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/semantic/state_inline_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../tests/unit/viewport/semantic/state_test_support.rs"]
mod test_support;
