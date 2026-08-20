use viewport_protocol::{SemanticPropertyScope, SessionId};

use super::super::lifecycle::{
    RuntimeMailbox, RuntimeMailboxWorkerGuard, TursoClientSyncRuntime,
    TursoClientSyncRuntimeCommand, TursoClientSyncRuntimeSubmitError,
};
use super::super::provisioning::TursoClientSyncProvisionRequest;
use super::policy;

#[test]
fn runtime_mailbox_close_unblocks_worker_and_returns_none() {
    let mailbox = RuntimeMailbox::new();
    let worker_mailbox = mailbox.clone();

    let handle = std::thread::spawn(move || {
        let mut received = Vec::new();
        while let Some(cmd) = worker_mailbox.pop() {
            received.push(cmd);
        }
        received
    });

    mailbox
        .submit(TursoClientSyncRuntimeCommand::Connect(SessionId::new(
            "s-1",
        )))
        .unwrap();

    std::thread::sleep(std::time::Duration::from_millis(20));
    mailbox.close();

    let received = handle.join().expect("worker thread should join cleanly");
    assert_eq!(received.len(), 1);
    assert_eq!(
        received[0],
        TursoClientSyncRuntimeCommand::Connect(SessionId::new("s-1"))
    );

    assert_eq!(
        mailbox.submit(TursoClientSyncRuntimeCommand::Connect(SessionId::new(
            "s-2"
        ))),
        Err(TursoClientSyncRuntimeSubmitError::WorkerUnavailable)
    );
}

#[test]
fn runtime_mailbox_worker_exit_marks_mailbox_closed_and_rejects_submits() {
    let mailbox = RuntimeMailbox::new();
    let worker_mailbox = mailbox.clone();

    let handle = std::thread::spawn(move || {
        let _worker_guard = RuntimeMailboxWorkerGuard {
            mailbox: worker_mailbox,
        };
    });

    handle.join().expect("worker thread should join cleanly");

    let normal_cmd = TursoClientSyncRuntimeCommand::Connect(SessionId::new("s-1"));
    assert_eq!(
        mailbox.submit(normal_cmd),
        Err(TursoClientSyncRuntimeSubmitError::WorkerUnavailable)
    );

    let control_cmd = TursoClientSyncRuntimeCommand::Close(SessionId::new("s-1"));
    assert_eq!(
        mailbox.submit(control_cmd),
        Err(TursoClientSyncRuntimeSubmitError::WorkerUnavailable)
    );
}

#[test]
fn runtime_drop_closes_mailbox_and_wakes_blocked_worker() {
    let mailbox = RuntimeMailbox::new();
    let worker_mailbox = mailbox.clone();
    let runtime = TursoClientSyncRuntime { mailbox };

    let handle = std::thread::spawn(move || {
        let worker_guard = RuntimeMailboxWorkerGuard {
            mailbox: worker_mailbox,
        };
        let mut received = Vec::new();
        while let Some(cmd) = worker_guard.mailbox.pop() {
            received.push(cmd);
        }
        received
    });

    runtime
        .submit(TursoClientSyncRuntimeCommand::Connect(SessionId::new(
            "s-1",
        )))
        .unwrap();

    std::thread::sleep(std::time::Duration::from_millis(20));
    drop(runtime);

    let received = handle
        .join()
        .expect("worker thread should join cleanly without leaking");
    assert_eq!(received.len(), 1);
    assert_eq!(
        received[0],
        TursoClientSyncRuntimeCommand::Connect(SessionId::new("s-1"))
    );
}

#[test]
fn runtime_mailbox_closed_session_rejects_subsequent_normal_commands() {
    let mailbox = RuntimeMailbox::new();
    let session_a = SessionId::new("session-a");
    let session_b = SessionId::new("session-b");

    mailbox
        .submit(TursoClientSyncRuntimeCommand::Close(session_a.clone()))
        .expect("Close(A) must be accepted");

    let prov_a = TursoClientSyncRuntimeCommand::Provision(TursoClientSyncProvisionRequest {
        session_id: session_a.clone(),
        client_name: "client-a".to_owned(),
        authorization: policy(SemanticPropertyScope::None, false),
    });
    assert_eq!(
        mailbox.submit(prov_a.clone()),
        Err(TursoClientSyncRuntimeSubmitError::SessionClosed)
    );

    let popped_close = mailbox.pop();
    assert!(popped_close.is_some());
    assert_eq!(
        mailbox.submit(prov_a),
        Err(TursoClientSyncRuntimeSubmitError::SessionClosed)
    );

    assert_eq!(
        mailbox.submit(TursoClientSyncRuntimeCommand::Connect(session_a.clone())),
        Err(TursoClientSyncRuntimeSubmitError::SessionClosed)
    );
    assert_eq!(
        mailbox.submit(TursoClientSyncRuntimeCommand::PullProjection(
            session_a.clone()
        )),
        Err(TursoClientSyncRuntimeSubmitError::SessionClosed)
    );

    let prov_b = TursoClientSyncRuntimeCommand::Provision(TursoClientSyncProvisionRequest {
        session_id: session_b.clone(),
        client_name: "client-b".to_owned(),
        authorization: policy(SemanticPropertyScope::None, false),
    });
    assert_eq!(mailbox.submit(prov_b), Ok(()));
}
