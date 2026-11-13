use std::collections::HashMap;

use usd_model::{HashDigest, SemanticSnapshot, SnapshotId};

use super::*;

fn snapshot() -> SemanticSnapshot {
    SemanticSnapshot {
        snapshot_id: SnapshotId("test".to_owned()),
        source: SnapshotSource::Working {
            session: "test".to_owned(),
            live_revision: 9,
        },
        config_hash: HashDigest::new([0; 32]),
        entities: HashMap::new(),
    }
}

fn update(revision: u64) -> SemanticIncrementalUpdate {
    SemanticIncrementalUpdate {
        snapshot_id: SnapshotId("test".to_owned()),
        source: SnapshotSource::Working {
            session: "test".to_owned(),
            live_revision: revision,
        },
        config_hash: HashDigest::new([0; 32]),
        upserts: Vec::new(),
        removed_paths: Vec::new(),
    }
}

#[test]
fn state_saturation_replaces_pending_deltas_with_latest_snapshot() {
    let mailbox = SemanticMailbox::new();
    for revision in 1..=STATE_MAILBOX_CAPACITY {
        assert_eq!(
            mailbox
                .submit_delta(format!("delta-{revision}"), update(revision as u64))
                .unwrap(),
            ()
        );
    }
    assert!(
        mailbox
            .submit_delta_with_snapshot("recovery".to_owned(), update(99), Arc::new(snapshot()),)
            .unwrap()
    );
    let command = mailbox.recv().expect("recovery command");
    assert!(matches!(
        command,
        SemanticMailboxCommand::State(SemanticStateCommand::ReplaceSnapshot { request_id, .. })
            if request_id == "recovery"
    ));
    assert_eq!(mailbox.stats().2, 1);
}
