use super::*;
use viewport_protocol::{AuthorizationPolicy, SemanticSyncOperation, SessionId};
use viewport_streaming::{SemanticSyncRequest, SemanticSyncRequestKind};

#[test]
fn convert_semantic_sync_request_converts_all_operations_and_controls() {
    let session_id = SessionId::new("session-test");
    let auth_policy = AuthorizationPolicy::stream_only();

    // 1. AuthorizationChanged
    let auth_req = SemanticSyncRequest {
        request_id: "req-auth-1".to_owned(),
        session_id: session_id.clone(),
        client_name: "test-client".to_owned(),
        authorization: auth_policy.clone(),
        kind: SemanticSyncRequestKind::AuthorizationChanged,
    };
    let (cmd, is_close) = convert_semantic_sync_request(auth_req, None).unwrap();
    assert!(!is_close);
    assert_eq!(
        cmd,
        TursoClientSyncRuntimeCommand::UpdateAuthorization {
            session_id: session_id.clone(),
            authorization: auth_policy.clone(),
        }
    );

    // 2. Close
    let close_req = SemanticSyncRequest {
        request_id: "req-close-1".to_owned(),
        session_id: session_id.clone(),
        client_name: "test-client".to_owned(),
        authorization: auth_policy.clone(),
        kind: SemanticSyncRequestKind::Client(SemanticSyncOperation::Close),
    };
    let (cmd_close, is_close) = convert_semantic_sync_request(close_req, None).unwrap();
    assert!(is_close);
    assert_eq!(
        cmd_close,
        TursoClientSyncRuntimeCommand::Close(session_id.clone())
    );

    // 3. Provision
    let prov_req = SemanticSyncRequest {
        request_id: "req-prov-1".to_owned(),
        session_id: session_id.clone(),
        client_name: "test-client".to_owned(),
        authorization: auth_policy.clone(),
        kind: SemanticSyncRequestKind::Client(SemanticSyncOperation::Provision),
    };
    let (cmd_prov, is_close) = convert_semantic_sync_request(prov_req, None).unwrap();
    assert!(!is_close);
    assert_eq!(
        cmd_prov,
        TursoClientSyncRuntimeCommand::Provision(TursoClientSyncProvisionRequest {
            session_id: session_id.clone(),
            client_name: "test-client".to_owned(),
            authorization: auth_policy.clone(),
        })
    );
}
