use viewport_protocol::{
    AuthorizationPolicy, SemanticSyncOperation, ServerCapabilities, ServerEvent, SessionEvent,
    SessionId, ViewportReadModel,
};

use crate::application::{
    MAX_PENDING_MESSAGES, RenderServerInterface, SemanticSyncRequest, SemanticSyncRequestKind,
};
use crate::data_channel::channel_set::ChannelOptions;
use crate::data_channel::constants::{CONTROL_CHANNEL_PROTOCOL, INPUT_CHANNEL_PROTOCOL};
use crate::data_channel::session::{ApplicationSession, remember_request_id};
use crate::session::SessionAdmission;

fn native_policy() -> AuthorizationPolicy {
    AuthorizationPolicy {
        allowed_delivery_modes: vec![viewport_protocol::DeliveryMode::SelfRender],
        model_download: viewport_protocol::ModelDownloadPermission::Allowed,
        allowed_blob_ids: vec![
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
        ],
        semantic_property_scope: viewport_protocol::SemanticPropertyScope::None,
        history: viewport_protocol::HistoryPermission::Denied,
        runtime_profile: viewport_protocol::RuntimeProfile::NativeMedium,
    }
}

#[test]
fn control_channel_is_ordered_and_reliable_by_default() {
    gstreamer::init().unwrap();
    let options = ChannelOptions::control();
    let structure = options.to_gstreamer_options();

    assert!(structure.get::<bool>("ordered").unwrap());
    assert_eq!(
        structure.get::<String>("protocol").unwrap(),
        CONTROL_CHANNEL_PROTOCOL
    );
    assert!(structure.get::<i32>("max-retransmits").is_err());
}

#[test]
fn input_channel_is_unordered_and_drop_eligible() {
    gstreamer::init().unwrap();
    let options = ChannelOptions::input();
    let structure = options.to_gstreamer_options();

    assert!(!structure.get::<bool>("ordered").unwrap());
    assert_eq!(structure.get::<i32>("max-retransmits").unwrap(), 0);
    assert_eq!(
        structure.get::<String>("protocol").unwrap(),
        INPUT_CHANNEL_PROTOCOL
    );
}

#[test]
fn recent_request_ids_are_deduplicated() {
    let session = ApplicationSession::new(
        SessionId::new("session-1"),
        ViewportReadModel::unloaded("stage.usda"),
        RenderServerInterface::default(),
    );
    let mut state = session.state.lock().unwrap();

    assert!(remember_request_id(&mut state, "request-1".to_owned()));
    assert!(!remember_request_id(&mut state, "request-1".to_owned()));
}

#[test]
fn disconnect_produces_internal_close_and_handles_queue_saturation() {
    let interface = RenderServerInterface::default();
    let session_id = SessionId::new("session-dc");
    let session = ApplicationSession::new_with_capabilities(
        session_id.clone(),
        ViewportReadModel::unloaded("stage.usda"),
        interface.clone(),
        ServerCapabilities::default(),
        AuthorizationPolicy::default(),
        SessionAdmission::default(),
    );

    {
        let mut state = session.state.lock().unwrap();
        state.handshaken = true;
        state.client_name = "test-client".to_owned();
    }

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

    session.release_admission();

    let popped = interface
        .pop_semantic_sync_request()
        .expect("control close must be popped first");
    assert_eq!(popped.session_id, session_id);
    assert_eq!(
        popped.kind,
        SemanticSyncRequestKind::Client(SemanticSyncOperation::Close)
    );
}

#[test]
fn authorization_refresh_produces_control_event_with_newest_policy() {
    let interface = RenderServerInterface::default();
    let session_id = SessionId::new("session-auth");
    let session = ApplicationSession::new_with_capabilities(
        session_id.clone(),
        ViewportReadModel::unloaded("stage.usda"),
        interface.clone(),
        ServerCapabilities::default(),
        AuthorizationPolicy::default(),
        SessionAdmission::default(),
    );

    {
        let mut state = session.state.lock().unwrap();
        state.handshaken = true;
        state.client_name = "test-client".to_owned();
    }

    let new_policy = native_policy();
    interface
        .publish_authorization_policy(new_policy.clone())
        .unwrap();

    session.refresh_authorization();

    let popped = interface
        .pop_semantic_sync_request()
        .expect("control authorization change must be queued");
    assert_eq!(popped.session_id, session_id);
    assert_eq!(popped.authorization, new_policy);
    assert_eq!(popped.kind, SemanticSyncRequestKind::AuthorizationChanged);

    let state = session.state.lock().unwrap();
    assert_eq!(state.authorization, new_policy);
    let last_event = state
        .pending_server_events
        .back()
        .expect("authorization change event must be queued for client");
    assert_eq!(
        last_event.event,
        ServerEvent::Session(SessionEvent::AuthorizationChanged {
            authorization: new_policy
        })
    );
}

#[test]
fn auth_downgrade_updates_session_authorization_and_blocks_further_sync() {
    let interface = RenderServerInterface::default();
    let session_id = SessionId::new("session-downgrade");
    let initial_policy = native_policy();
    let session = ApplicationSession::new_with_capabilities(
        session_id.clone(),
        ViewportReadModel::unloaded("stage.usda"),
        interface.clone(),
        ServerCapabilities::default(),
        initial_policy.clone(),
        SessionAdmission::default(),
    );

    {
        let mut state = session.state.lock().unwrap();
        state.handshaken = true;
        state.client_name = "test-client".to_owned();
    }

    let restrictive_policy = AuthorizationPolicy::default();
    interface
        .publish_authorization_policy(restrictive_policy.clone())
        .unwrap();
    session.refresh_authorization();

    let state = session.state.lock().unwrap();
    assert_eq!(state.authorization, restrictive_policy);
    assert!(!state.authorization.allows_self_render_delivery());
    assert!(!state.authorization.allows_model_download());
}

#[test]
fn repeat_disconnect_is_idempotent_and_safe() {
    let interface = RenderServerInterface::default();
    let session_id = SessionId::new("session-repeat-dc");
    let session = ApplicationSession::new_with_capabilities(
        session_id.clone(),
        ViewportReadModel::unloaded("stage.usda"),
        interface.clone(),
        ServerCapabilities::default(),
        AuthorizationPolicy::default(),
        SessionAdmission::default(),
    );

    {
        let mut state = session.state.lock().unwrap();
        state.handshaken = true;
        state.client_name = "test-client".to_owned();
    }

    session.release_admission();
    session.release_admission();

    let popped = interface
        .pop_semantic_sync_request()
        .expect("Close should be queued");
    assert_eq!(popped.session_id, session_id);
    assert_eq!(
        popped.kind,
        SemanticSyncRequestKind::Client(SemanticSyncOperation::Close)
    );
    assert!(interface.pop_semantic_sync_request().is_none());
}
