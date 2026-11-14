use super::*;

impl SessionAdmission {
    fn role(&self, session_id: &SessionId) -> Option<SessionRole> {
        self.state
            .lock()
            .expect("session admission state is not poisoned")
            .roles
            .get(session_id)
            .copied()
    }
}

#[test]
fn one_controller_and_multiple_observers_are_admitted() {
    let admission = SessionAdmission::default();
    let controller = SessionId::new("controller");
    let observer_a = SessionId::new("observer-a");
    let observer_b = SessionId::new("observer-b");

    admission
        .register(controller.clone(), SessionRole::Controller)
        .unwrap();
    admission
        .register(observer_a.clone(), SessionRole::Observer)
        .unwrap();
    admission
        .register(observer_b.clone(), SessionRole::Observer)
        .unwrap();

    assert_eq!(admission.role(&controller), Some(SessionRole::Controller));
    assert_eq!(admission.role(&observer_a), Some(SessionRole::Observer));
    assert_eq!(admission.role(&observer_b), Some(SessionRole::Observer));
}

#[test]
fn second_controller_is_rejected_without_replacing_the_first() {
    let admission = SessionAdmission::default();
    let first = SessionId::new("controller-1");
    admission
        .register(first.clone(), SessionRole::Controller)
        .unwrap();

    assert_eq!(
        admission.register(SessionId::new("controller-2"), SessionRole::Controller),
        Err(SessionAdmissionError::ControllerAlreadyAssigned)
    );
    assert_eq!(admission.role(&first), Some(SessionRole::Controller));
}

#[test]
fn unregistering_controller_allows_a_new_controller() {
    let admission = SessionAdmission::default();
    let first = SessionId::new("controller-1");
    admission
        .register(first.clone(), SessionRole::Controller)
        .unwrap();
    assert!(admission.unregister(&first));
    admission
        .register(SessionId::new("controller-2"), SessionRole::Controller)
        .unwrap();
}
