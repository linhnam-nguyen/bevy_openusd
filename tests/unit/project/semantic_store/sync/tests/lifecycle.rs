use viewport_protocol::{AuthorizationPolicy, SemanticPropertyScope, SemanticSyncPhase, SessionId};

use super::super::coordinator::TursoClientSyncCoordinator;
use super::super::provisioning::TursoClientSyncProvisionRequest;
use super::{RecordingProvisioner, policy};

#[test]
fn coordinator_reprovisions_fresh_lease_after_stale_or_closed() {
    let provisioner = RecordingProvisioner::default();
    let provisioned = provisioner.provisioned.clone();
    let revoked = provisioner.revoked.clone();
    let mut coordinator = TursoClientSyncCoordinator::new(provisioner);
    let session_id = SessionId::new("session-lifecycle");

    let request = TursoClientSyncProvisionRequest {
        session_id: session_id.clone(),
        client_name: "client".to_owned(),
        authorization: policy(SemanticPropertyScope::None, false),
    };

    // 1. Initial provision
    coordinator.provision(request.clone()).unwrap();
    assert_eq!(
        coordinator.status(&session_id).unwrap().phase,
        SemanticSyncPhase::Provisioned
    );
    assert_eq!(provisioned.lock().unwrap().len(), 1);

    // 2. Invalidate via authorization downgrade -> Stale (revoked once)
    coordinator
        .update_authorization(&session_id, AuthorizationPolicy::default())
        .unwrap();
    assert_eq!(
        coordinator.status(&session_id).unwrap().phase,
        SemanticSyncPhase::Stale
    );
    assert_eq!(revoked.lock().unwrap().len(), 1);

    // 3. Re-provision after Stale -> fresh lease obtained
    coordinator.provision(request.clone()).unwrap();
    assert_eq!(
        coordinator.status(&session_id).unwrap().phase,
        SemanticSyncPhase::Provisioned
    );
    assert_eq!(provisioned.lock().unwrap().len(), 2);

    // 4. Close session -> Closed (revoked once more)
    coordinator.close(&session_id).unwrap();
    assert_eq!(revoked.lock().unwrap().len(), 2);
    assert_eq!(
        coordinator.drain_updates().last().unwrap().status.phase,
        SemanticSyncPhase::Closed
    );

    // 5. Re-provision new lifecycle under same session ID -> fresh lease
    coordinator.provision(request).unwrap();
    assert_eq!(
        coordinator.status(&session_id).unwrap().phase,
        SemanticSyncPhase::Provisioned
    );
    assert_eq!(provisioned.lock().unwrap().len(), 3);
}
