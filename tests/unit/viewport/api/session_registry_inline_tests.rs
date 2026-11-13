use super::*;

#[test]
fn only_one_controller_can_be_registered() {
    let mut registry = SessionRegistry::default();
    registry
        .register(SessionId::new("controller-1"), SessionRole::Controller)
        .unwrap();

    assert_eq!(
        registry.register(SessionId::new("controller-2"), SessionRole::Controller),
        Err(SessionRegistryError::ControllerAlreadyAssigned)
    );
}

#[test]
fn unregistering_an_observer_does_not_release_the_controller() {
    let mut registry = SessionRegistry::default();
    let controller = SessionId::new("controller");
    let observer = SessionId::new("observer");
    registry
        .register(controller.clone(), SessionRole::Controller)
        .unwrap();
    registry
        .register(observer.clone(), SessionRole::Observer)
        .unwrap();

    assert!(registry.unregister(&observer));
    assert_eq!(registry.role(&controller), Some(SessionRole::Controller));
    assert_eq!(
        registry.register(SessionId::new("controller-2"), SessionRole::Controller),
        Err(SessionRegistryError::ControllerAlreadyAssigned)
    );
}
