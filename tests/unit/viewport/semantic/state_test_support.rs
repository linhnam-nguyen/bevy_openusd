//! Test-only helpers attached to the owning production module.
use super::*;

impl SemanticSyncState {
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
