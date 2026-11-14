use viewport_protocol::{
    AuthorizationPolicy, SemanticSyncOperation, SemanticSyncPhase, SemanticSyncStatus, SessionId,
};
use viewport_streaming::RenderServerInterface;
use viewport_streaming::{RenderServerPortError, SemanticSyncRequest, SemanticSyncRequestKind};

#[test]
fn runtime_delivery_registry_rejects_invalid_ids_and_keeps_valid_payloads() {
    let interface = RenderServerInterface::default();
    assert_eq!(
        interface.publish_runtime_blob("../outside", vec![1, 2, 3]),
        Err(RenderServerPortError::InvalidPayload)
    );

    let blob_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    interface
        .publish_runtime_blob(blob_id, vec![1, 2, 3])
        .unwrap();
    assert_eq!(interface.runtime_blob(blob_id), Some(vec![1, 2, 3]));
}

#[test]
fn authorization_policy_is_validated_and_replaceable() {
    let interface = RenderServerInterface::default();
    assert_eq!(
        interface.authorization_policy(),
        AuthorizationPolicy::default()
    );

    let policy = AuthorizationPolicy {
        semantic_property_scope: viewport_protocol::SemanticPropertyScope::All,
        ..Default::default()
    };
    interface
        .publish_authorization_policy(policy.clone())
        .unwrap();

    assert_eq!(interface.authorization_policy(), policy);
    assert_eq!(
        interface.publish_authorization_policy(AuthorizationPolicy {
            allowed_delivery_modes: Vec::new(),
            ..AuthorizationPolicy::default()
        }),
        Err(RenderServerPortError::InvalidPayload)
    );
    assert_eq!(interface.authorization_policy(), policy);
}

#[test]
fn semantic_sync_status_is_replaced_per_session_and_can_be_cleared() {
    let interface = RenderServerInterface::default();
    let session_id = SessionId::new("session-sync");
    let ready = SemanticSyncStatus::ready("working-7".to_owned(), "hash-1".to_owned());
    interface
        .publish_semantic_sync_status(session_id.clone(), ready.clone())
        .unwrap();
    assert_eq!(interface.semantic_sync_status(&session_id), Some(ready));

    let stale = SemanticSyncStatus::phase(
        SemanticSyncPhase::Stale,
        Some("authorization_changed".to_owned()),
    );
    interface
        .publish_semantic_sync_status(session_id.clone(), stale.clone())
        .unwrap();
    assert_eq!(interface.semantic_sync_status(&session_id), Some(stale));

    interface.clear_semantic_sync_status(&session_id);
    assert!(interface.semantic_sync_status(&session_id).is_none());
}

#[test]
fn semantic_sync_requests_preserve_server_context_and_reject_invalid_context() {
    let interface = RenderServerInterface::default();
    let request = SemanticSyncRequest {
        request_id: "sync-1".to_owned(),
        session_id: SessionId::new("session-sync"),
        client_name: "native-client".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::Client(SemanticSyncOperation::Provision),
    };
    interface
        .submit_semantic_sync_request(request.clone())
        .expect("valid semantic-sync request should queue");
    assert_eq!(interface.pop_semantic_sync_request(), Some(request.clone()));

    let invalid = SemanticSyncRequest {
        client_name: String::new(),
        ..request
    };
    assert_eq!(
        interface.submit_semantic_sync_request(invalid),
        Err(RenderServerPortError::InvalidPayload)
    );
}
