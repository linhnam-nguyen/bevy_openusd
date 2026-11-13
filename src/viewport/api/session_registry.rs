//! Application-session ownership and controller policy.

#![allow(dead_code)]

use std::collections::HashMap;
use viewport_protocol::{SessionId, SessionRole};

use bevy::prelude::Resource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRegistryError {
    ControllerAlreadyAssigned,
    SessionAlreadyRegistered,
}

#[derive(Resource, Debug, Default)]
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
#[path = "../../../tests/unit/viewport/api/session_registry_inline_tests.rs"]
mod tests;
