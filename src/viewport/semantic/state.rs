use bevy::prelude::Resource;
use usd_bevy::LiveRevision;
use usd_model::SemanticSnapshot;

/// Local authoritative semantic state used to derive the next incremental
/// update from the same live-stage revision consumed by Bevy projection.
#[derive(Resource, Default)]
pub(crate) struct SemanticSyncState {
    pub(super) snapshot: Option<SemanticSnapshot>,
    pub(super) session_id: Option<u64>,
    pub(super) revision: Option<LiveRevision>,
}

impl SemanticSyncState {
    pub(crate) fn snapshot(&self) -> Option<&SemanticSnapshot> {
        self.snapshot.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn from_test_snapshot(snapshot: SemanticSnapshot) -> Self {
        Self {
            snapshot: Some(snapshot),
            session_id: None,
            revision: None,
        }
    }
}
