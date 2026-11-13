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
