use viewport_protocol::*;

#[test]
fn semantic_sync_status_constructors_and_serialization_match_protocol() {
    let disabled = SemanticSyncStatus::disabled();
    assert_eq!(disabled.phase, SemanticSyncPhase::Disabled);
    assert!(disabled.source_snapshot_id.is_none());
    assert!(disabled.projection_hash.is_none());
    assert!(disabled.detail.is_none());

    let json = serde_json::to_string(&disabled).unwrap();
    assert_eq!(json, r#"{"phase":"disabled"}"#);

    let failed =
        SemanticSyncStatus::phase(SemanticSyncPhase::Failed, Some("revoke_failed".to_owned()));
    let json_failed = serde_json::to_string(&failed).unwrap();
    assert_eq!(
        json_failed,
        r#"{"phase":"failed","detail":"revoke_failed"}"#
    );

    let ready = SemanticSyncStatus::ready("snap-1".to_owned(), "hash-abc".to_owned());
    let json_ready = serde_json::to_string(&ready).unwrap();
    assert_eq!(
        json_ready,
        r#"{"phase":"ready","source_snapshot_id":"snap-1","projection_hash":"hash-abc"}"#
    );

    let decoded: SemanticSyncStatus = serde_json::from_str(&json_ready).unwrap();
    assert_eq!(decoded, ready);
}

#[test]
fn semantic_sync_phases_serialize_to_snake_case() {
    for (phase, expected) in [
        (SemanticSyncPhase::Disabled, "disabled"),
        (SemanticSyncPhase::Provisioning, "provisioning"),
        (SemanticSyncPhase::Provisioned, "provisioned"),
        (SemanticSyncPhase::Connecting, "connecting"),
        (SemanticSyncPhase::Ready, "ready"),
        (SemanticSyncPhase::Pulling, "pulling"),
        (SemanticSyncPhase::Pushing, "pushing"),
        (SemanticSyncPhase::Stale, "stale"),
        (SemanticSyncPhase::Failed, "failed"),
        (SemanticSyncPhase::Closed, "closed"),
    ] {
        let json = serde_json::to_string(&phase).unwrap();
        assert_eq!(json, format!("\"{expected}\""));
    }
}
