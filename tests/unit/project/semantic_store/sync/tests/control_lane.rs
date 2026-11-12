use viewport_protocol::{AuthorizationPolicy, SemanticPropertyScope, SessionId};

use super::super::lifecycle::{
    MAX_PENDING_SYNC_RUNTIME_COMMANDS, RuntimeMailbox, TursoClientSyncRuntimeCommand,
    TursoClientSyncRuntimeSubmitError,
};
use super::super::provisioning::TursoClientSyncProvisionRequest;
use super::policy;

#[test]
fn runtime_mailbox_enforces_capacity_and_prioritizes_controls() {
    let mailbox = RuntimeMailbox::new();
    let session_a = SessionId::new("session-a");
    let session_b = SessionId::new("session-b");

    // Fill normal capacity
    for i in 0..MAX_PENDING_SYNC_RUNTIME_COMMANDS {
        let cmd = TursoClientSyncRuntimeCommand::Connect(SessionId::new(format!("s-{i}")));
        mailbox.submit(cmd).expect("should submit up to capacity");
    }

    // Next normal command fails with QueueFull without blocking
    let overflow = TursoClientSyncRuntimeCommand::Connect(SessionId::new("s-overflow"));
    assert_eq!(
        mailbox.submit(overflow),
        Err(TursoClientSyncRuntimeSubmitError::QueueFull)
    );

    // Control command for session A is accepted despite full normal queue
    let auth_a = TursoClientSyncRuntimeCommand::UpdateAuthorization {
        session_id: session_a.clone(),
        authorization: AuthorizationPolicy::default(),
    };
    mailbox
        .submit(auth_a.clone())
        .expect("control command must be accepted when normal is full");

    // Close for session B is accepted despite full normal queue
    let close_b = TursoClientSyncRuntimeCommand::Close(session_b.clone());
    mailbox
        .submit(close_b.clone())
        .expect("Close control must be accepted when normal is full");

    // First pop must be a control command
    let first_pop = mailbox.pop().expect("pop should return control");
    assert!(first_pop.is_control());
}

#[test]
fn runtime_mailbox_control_precedence_and_purges_same_session() {
    let mailbox = RuntimeMailbox::new();
    let session_a = SessionId::new("session-a");
    let session_b = SessionId::new("session-b");

    // Queue normal commands for session A and B
    let prov_a = TursoClientSyncRuntimeCommand::Provision(TursoClientSyncProvisionRequest {
        session_id: session_a.clone(),
        client_name: "client-a".to_owned(),
        authorization: policy(SemanticPropertyScope::None, false),
    });
    let conn_a = TursoClientSyncRuntimeCommand::Connect(session_a.clone());
    let conn_b = TursoClientSyncRuntimeCommand::Connect(session_b.clone());

    mailbox.submit(prov_a).unwrap();
    mailbox.submit(conn_a).unwrap();
    mailbox.submit(conn_b.clone()).unwrap();

    // Submit UpdateAuthorization for A -> purges normal A work (prov_a, conn_a)
    let auth_a = TursoClientSyncRuntimeCommand::UpdateAuthorization {
        session_id: session_a.clone(),
        authorization: AuthorizationPolicy::default(),
    };
    mailbox.submit(auth_a).unwrap();

    // Submit Close for A -> supersedes UpdateAuthorization for A
    let close_a = TursoClientSyncRuntimeCommand::Close(session_a.clone());
    mailbox.submit(close_a.clone()).unwrap();

    // Subsequent UpdateAuthorization for A cannot supersede Close
    let auth_a2 = TursoClientSyncRuntimeCommand::UpdateAuthorization {
        session_id: session_a.clone(),
        authorization: AuthorizationPolicy::default(),
    };
    mailbox.submit(auth_a2).unwrap();

    // Pop 1: Close for A (control)
    assert_eq!(mailbox.pop(), Some(close_a));
    // Pop 2: Connect for B (normal B was NOT purged)
    assert_eq!(mailbox.pop(), Some(conn_b));
}

#[test]
fn runtime_mailbox_update_authorization_allows_later_normal_commands() {
    let mailbox = RuntimeMailbox::new();
    let session_a = SessionId::new("session-a");

    let auth_a = TursoClientSyncRuntimeCommand::UpdateAuthorization {
        session_id: session_a.clone(),
        authorization: AuthorizationPolicy::default(),
    };
    mailbox
        .submit(auth_a)
        .expect("UpdateAuthorization(A) must be accepted");

    // Later normal command for A is allowed (not closed)
    let prov_a = TursoClientSyncRuntimeCommand::Provision(TursoClientSyncProvisionRequest {
        session_id: session_a.clone(),
        client_name: "client-a".to_owned(),
        authorization: policy(SemanticPropertyScope::None, false),
    });
    assert_eq!(mailbox.submit(prov_a), Ok(()));
}
