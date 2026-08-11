//! Application-session ownership and controller policy.

#![allow(dead_code)]

use std::collections::HashMap;
use viewport_protocol::{SessionId, SessionRole};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRegistryError {
    ControllerAlreadyAssigned,
    SessionAlreadyRegistered,
}

#[derive(Debug, Default)]
pub struct SessionRegistry {
    sessions: HashMap<SessionId, SessionRole>,
    controller: Option<SessionId>,
}

impl SessionRegistry {
    pub fn register(
        &mut self,
        session_id: SessionId,
        role: SessionRole,
    ) -> Result<(), SessionRegistryError> {
        if self.sessions.contains_key(&session_id) {
            return Err(SessionRegistryError::SessionAlreadyRegistered);
        }
        if role == SessionRole::Controller && self.controller.is_some() {
            return Err(SessionRegistryError::ControllerAlreadyAssigned);
        }

        if role == SessionRole::Controller {
            self.controller = Some(session_id.clone());
        }
        self.sessions.insert(session_id, role);
        Ok(())
    }

    pub fn unregister(&mut self, session_id: &SessionId) -> bool {
        let removed = self.sessions.remove(session_id).is_some();
        if self.controller.as_ref() == Some(session_id) {
            self.controller = None;
        }
        removed
    }

    pub fn role(&self, session_id: &SessionId) -> Option<SessionRole> {
        self.sessions.get(session_id).copied()
    }
}

#[cfg(test)]
mod tests {
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
}
