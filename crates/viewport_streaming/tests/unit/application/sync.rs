use crate::application::interface::RenderServerInterface;
use crate::application::types::{
    MAX_PENDING_MESSAGES, RenderServerPortError, SemanticSyncRequest, SemanticSyncRequestKind,
};
use viewport_protocol::{AuthorizationPolicy, SemanticSyncOperation, SessionId};

#[test]
fn semantic_sync_control_request_accepted_when_normal_queue_is_full() {
    let interface = RenderServerInterface::default();
    for i in 0..MAX_PENDING_MESSAGES {
        let req = SemanticSyncRequest {
            request_id: format!("req-{i}"),
            session_id: SessionId::new(format!("session-{i}")),
            client_name: "client".to_owned(),
            authorization: AuthorizationPolicy::default(),
            kind: SemanticSyncRequestKind::Client(SemanticSyncOperation::Provision),
        };
        interface
            .submit_semantic_sync_request(req)
            .expect("queueing normal request");
    }

    let extra_normal = SemanticSyncRequest {
        request_id: "req-overflow".to_owned(),
        session_id: SessionId::new("session-overflow"),
        client_name: "client".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::Client(SemanticSyncOperation::Provision),
    };
    assert_eq!(
        interface.submit_semantic_sync_request(extra_normal),
        Err(RenderServerPortError::QueueFull)
    );

    let control_req = SemanticSyncRequest {
        request_id: "close-ctrl".to_owned(),
        session_id: SessionId::new("session-ctrl"),
        client_name: "client".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::Client(SemanticSyncOperation::Close),
    };
    interface
        .submit_semantic_sync_control_request(control_req.clone())
        .expect("control request must succeed when normal queue is full");

    assert_eq!(interface.pop_semantic_sync_request(), Some(control_req));
}

#[test]
fn semantic_sync_control_latest_authorization_changed_wins() {
    let interface = RenderServerInterface::default();
    let session_a = SessionId::new("session-a");

    let auth_a1 = SemanticSyncRequest {
        request_id: "auth-a1".to_owned(),
        session_id: session_a.clone(),
        client_name: "client".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::AuthorizationChanged,
    };
    let auth_a2 = SemanticSyncRequest {
        request_id: "auth-a2".to_owned(),
        session_id: session_a.clone(),
        client_name: "client".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::AuthorizationChanged,
    };

    interface
        .submit_semantic_sync_control_request(auth_a1)
        .unwrap();
    interface
        .submit_semantic_sync_control_request(auth_a2.clone())
        .unwrap();

    assert_eq!(interface.pop_semantic_sync_request(), Some(auth_a2));
    assert_eq!(interface.pop_semantic_sync_request(), None);
}

#[test]
fn semantic_sync_control_close_remains_and_purges_normal_work_on_subsequent_auth_change() {
    let interface = RenderServerInterface::default();
    let session_a = SessionId::new("session-a");

    let provision_a = SemanticSyncRequest {
        request_id: "prov-a".to_owned(),
        session_id: session_a.clone(),
        client_name: "client".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::Client(SemanticSyncOperation::Provision),
    };
    interface.submit_semantic_sync_request(provision_a).unwrap();

    let close_a = SemanticSyncRequest {
        request_id: "close-a".to_owned(),
        session_id: session_a.clone(),
        client_name: "client".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::Client(SemanticSyncOperation::Close),
    };
    interface
        .submit_semantic_sync_control_request(close_a.clone())
        .unwrap();

    let auth_a = SemanticSyncRequest {
        request_id: "auth-a".to_owned(),
        session_id: session_a.clone(),
        client_name: "client".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::AuthorizationChanged,
    };
    interface
        .submit_semantic_sync_control_request(auth_a)
        .unwrap();

    assert_eq!(interface.pop_semantic_sync_request(), Some(close_a));
    assert_eq!(interface.pop_semantic_sync_request(), None);
}

#[test]
fn semantic_sync_control_precedence_and_purging() {
    let interface = RenderServerInterface::default();
    let session_a = SessionId::new("session-a");
    let session_b = SessionId::new("session-b");

    let normal_a = SemanticSyncRequest {
        request_id: "norm-a".to_owned(),
        session_id: session_a.clone(),
        client_name: "client".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::Client(SemanticSyncOperation::PushSnapshot),
    };
    let normal_b = SemanticSyncRequest {
        request_id: "norm-b".to_owned(),
        session_id: session_b.clone(),
        client_name: "client".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::Client(SemanticSyncOperation::PushSnapshot),
    };
    interface.submit_semantic_sync_request(normal_a).unwrap();
    interface
        .submit_semantic_sync_request(normal_b.clone())
        .unwrap();

    let auth_a1 = SemanticSyncRequest {
        request_id: "auth-a1".to_owned(),
        session_id: session_a.clone(),
        client_name: "client".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::AuthorizationChanged,
    };
    interface
        .submit_semantic_sync_control_request(auth_a1)
        .unwrap();

    let close_a = SemanticSyncRequest {
        request_id: "close-a".to_owned(),
        session_id: session_a.clone(),
        client_name: "client".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::Client(SemanticSyncOperation::Close),
    };
    interface
        .submit_semantic_sync_control_request(close_a.clone())
        .unwrap();

    let auth_a3 = SemanticSyncRequest {
        request_id: "auth-a3".to_owned(),
        session_id: session_a.clone(),
        client_name: "client".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::AuthorizationChanged,
    };
    interface
        .submit_semantic_sync_control_request(auth_a3)
        .unwrap();

    assert_eq!(interface.pop_semantic_sync_request(), Some(close_a));
    assert_eq!(interface.pop_semantic_sync_request(), Some(normal_b));
    assert_eq!(interface.pop_semantic_sync_request(), None);
}

#[test]
fn semantic_sync_control_method_rejects_non_control_operations() {
    let interface = RenderServerInterface::default();
    for op in [
        SemanticSyncOperation::Provision,
        SemanticSyncOperation::Connect,
        SemanticSyncOperation::PushSnapshot,
        SemanticSyncOperation::PullProjection,
    ] {
        let req = SemanticSyncRequest {
            request_id: "req".to_owned(),
            session_id: SessionId::new("session"),
            client_name: "client".to_owned(),
            authorization: AuthorizationPolicy::default(),
            kind: SemanticSyncRequestKind::Client(op),
        };
        assert_eq!(
            interface.submit_semantic_sync_control_request(req),
            Err(RenderServerPortError::InvalidPayload)
        );
    }
}

#[test]
fn closed_session_rejects_subsequent_normal_requests_at_application_queue() {
    let interface = RenderServerInterface::default();
    let session_a = SessionId::new("session-a");
    let session_b = SessionId::new("session-b");

    let close_a = SemanticSyncRequest {
        request_id: "close-a".to_owned(),
        session_id: session_a.clone(),
        client_name: "client-a".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::Client(SemanticSyncOperation::Close),
    };
    interface
        .submit_semantic_sync_control_request(close_a)
        .expect("Close(A) must be accepted");

    let prov_a = SemanticSyncRequest {
        request_id: "prov-a".to_owned(),
        session_id: session_a.clone(),
        client_name: "client-a".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::Client(SemanticSyncOperation::Provision),
    };
    assert_eq!(
        interface.submit_semantic_sync_request(prov_a.clone()),
        Err(RenderServerPortError::SessionClosed)
    );

    let popped_close = interface.pop_semantic_sync_request();
    assert!(popped_close.is_some());
    assert_eq!(
        interface.submit_semantic_sync_request(prov_a),
        Err(RenderServerPortError::SessionClosed)
    );

    for op in [
        SemanticSyncOperation::Connect,
        SemanticSyncOperation::PushSnapshot,
        SemanticSyncOperation::PullProjection,
    ] {
        let req = SemanticSyncRequest {
            request_id: "req-a".to_owned(),
            session_id: session_a.clone(),
            client_name: "client-a".to_owned(),
            authorization: AuthorizationPolicy::default(),
            kind: SemanticSyncRequestKind::Client(op),
        };
        assert_eq!(
            interface.submit_semantic_sync_request(req),
            Err(RenderServerPortError::SessionClosed)
        );
    }

    let prov_b = SemanticSyncRequest {
        request_id: "prov-b".to_owned(),
        session_id: session_b.clone(),
        client_name: "client-b".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::Client(SemanticSyncOperation::Provision),
    };
    assert_eq!(interface.submit_semantic_sync_request(prov_b), Ok(()));
}

#[test]
fn update_authorization_allows_later_normal_requests_at_application_queue() {
    let interface = RenderServerInterface::default();
    let session_a = SessionId::new("session-a");

    let auth_a = SemanticSyncRequest {
        request_id: "auth-a".to_owned(),
        session_id: session_a.clone(),
        client_name: "client-a".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::AuthorizationChanged,
    };
    interface
        .submit_semantic_sync_control_request(auth_a)
        .expect("UpdateAuthorization(A) must be accepted");

    let prov_a = SemanticSyncRequest {
        request_id: "prov-a".to_owned(),
        session_id: session_a.clone(),
        client_name: "client-a".to_owned(),
        authorization: AuthorizationPolicy::default(),
        kind: SemanticSyncRequestKind::Client(SemanticSyncOperation::Provision),
    };
    assert_eq!(interface.submit_semantic_sync_request(prov_a), Ok(()));
}
