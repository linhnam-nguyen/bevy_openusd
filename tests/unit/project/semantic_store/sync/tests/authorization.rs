use viewport_protocol::{AuthorizationPolicy, SemanticPropertyScope, SemanticSyncPhase, SessionId};

use super::super::coordinator::TursoClientSyncCoordinator;
use super::super::provisioning::TursoClientSyncProvisionRequest;
use super::{RecordingProvisioner, policy};

#[test]
fn coordinator_requires_self_render_before_provisioning() {
    let provisioner = RecordingProvisioner::default();
    let records = provisioner.provisioned.clone();
    let mut coordinator = TursoClientSyncCoordinator::new(provisioner);
    let session_id = SessionId::new("session-stream-only");
    let error = coordinator
        .provision(TursoClientSyncProvisionRequest {
            session_id: session_id.clone(),
            client_name: "client".to_owned(),
            authorization: AuthorizationPolicy::default(),
        })
        .expect_err("stream-only sessions must not receive semantic sync");

    assert!(error.to_string().contains("self-render"));
    assert!(records.lock().unwrap().is_empty());
    assert!(coordinator.status(&session_id).is_none());
}

#[test]
fn coordinator_tracks_provisioning_and_revokes_once_on_policy_change() {
    let provisioner = RecordingProvisioner::default();
    let revoked = provisioner.revoked.clone();
    let mut coordinator = TursoClientSyncCoordinator::new(provisioner);
    let session_id = SessionId::new("session-self-render");
    coordinator
        .provision(TursoClientSyncProvisionRequest {
            session_id: session_id.clone(),
            client_name: "client".to_owned(),
            authorization: policy(SemanticPropertyScope::None, false),
        })
        .expect("self-render session should provision");

    assert_eq!(
        coordinator.status(&session_id).unwrap().phase,
        SemanticSyncPhase::Provisioned
    );
    let updates = coordinator.drain_updates();
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].status.phase, SemanticSyncPhase::Provisioning);
    assert_eq!(updates[1].status.phase, SemanticSyncPhase::Provisioned);

    coordinator
        .update_authorization(&session_id, AuthorizationPolicy::default())
        .expect("policy change should revoke the old lease");
    assert_eq!(
        coordinator.status(&session_id).unwrap().phase,
        SemanticSyncPhase::Stale
    );
    assert_eq!(
        revoked.lock().unwrap().as_slice(),
        std::slice::from_ref(&session_id)
    );

    coordinator
        .close(&session_id)
        .expect("closing a stale session should not revoke twice");
    assert_eq!(revoked.lock().unwrap().as_slice(), &[session_id]);
    assert_eq!(
        coordinator.drain_updates().last().unwrap().status.phase,
        SemanticSyncPhase::Closed
    );
}

#[test]
fn coordinator_revoke_failure_emits_failed_status_with_detail() {
    let provisioner = RecordingProvisioner::default();
    *provisioner.fail_revoke.lock().unwrap() = true;
    let mut coordinator = TursoClientSyncCoordinator::new(provisioner);
    let session_id = SessionId::new("session-revoke-fail");

    coordinator
        .provision(TursoClientSyncProvisionRequest {
            session_id: session_id.clone(),
            client_name: "client".to_owned(),
            authorization: policy(SemanticPropertyScope::None, false),
        })
        .unwrap();

    let result = coordinator.update_authorization(&session_id, AuthorizationPolicy::default());
    assert!(result.is_err());
    let status = coordinator.status(&session_id).unwrap();
    assert_eq!(status.phase, SemanticSyncPhase::Failed);
    assert_eq!(status.detail.as_deref(), Some("revoke_failed"));
}
