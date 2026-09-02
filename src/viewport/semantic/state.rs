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

    #[cfg(test)]
    pub(crate) fn from_test_snapshot(snapshot: SemanticSnapshot) -> Self {
        Self {
            config: SemanticConfig::default(),
            bim_index: Some(Arc::new(BimReadIndex::build(&snapshot))),
            snapshot: Some(Arc::new(snapshot)),
            session_id: None,
            revision: None,
            activation_generation: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use usd_model::{HashDigest, SemanticSnapshot, SnapshotId, SnapshotSource};

    use super::SemanticSyncState;

    #[test]
    fn activation_reset_invalidates_snapshot_and_records_generation() {
        let snapshot = SemanticSnapshot {
            snapshot_id: SnapshotId("activation-reset".to_owned()),
            source: SnapshotSource::Working {
                session: "activation-reset".to_owned(),
                live_revision: 1,
            },
            config_hash: HashDigest::new([0; HashDigest::BYTE_LEN]),
            entities: HashMap::new(),
        };
        let mut state = SemanticSyncState::from_test_snapshot(snapshot);

        assert!(state.snapshot().is_some());
        assert!(state.shared_bim_index().is_some());
        state.reset_for_activation(42);

        assert!(state.snapshot().is_none());
        assert!(state.shared_bim_index().is_none());
        assert_eq!(state.activation_generation(), 42);
    }
}
