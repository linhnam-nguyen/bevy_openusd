//! Transport-neutral render-server application ports.
//!
//! Phase 1 reserves the application boundary without routing product commands
//! yet. Transport adapters must depend on these ports rather than on the Bevy
//! World or renderer resources directly.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::Mutex;

use bevy::prelude::Resource;
use viewport_protocol::{ClientCommandEnvelope, ServerEventEnvelope, SessionId};

const MAX_PENDING_MESSAGES: usize = 256;

#[derive(Debug, Default)]
struct PendingMessages {
    commands: VecDeque<ClientCommandEnvelope>,
    events: VecDeque<ServerEventEnvelope>,
}

/// Concrete application boundary shared by future transport adapters.
///
/// The queues are deliberately not connected to the viewport command systems
/// yet. Phase 1 only establishes ownership and bounded transport-facing
/// storage; command validation/routing and event publication belong to the
/// later application phases.
#[derive(Debug, Default, Resource)]
pub(crate) struct RenderServerInterface {
    pending: Mutex<PendingMessages>,
}

impl RenderServerInterface {
    pub(crate) fn pending_command_count(&self) -> usize {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .commands
            .len()
    }

    pub(crate) fn pending_event_count(&self) -> usize {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .events
            .len()
    }
}

pub trait RenderServerCommandPort {
    type Error;

    fn submit(&self, command: ClientCommandEnvelope) -> Result<(), Self::Error>;
}

pub trait RenderServerEventPort {
    type Error;

    fn next_event(
        &mut self,
        session_id: &SessionId,
    ) -> Result<Option<ServerEventEnvelope>, Self::Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderServerPortError {
    QueueClosed,
    QueueFull,
    InvalidPayload,
}

impl RenderServerCommandPort for RenderServerInterface {
    type Error = RenderServerPortError;

    fn submit(&self, command: ClientCommandEnvelope) -> Result<(), Self::Error> {
        command
            .validate()
            .map_err(|_| RenderServerPortError::InvalidPayload)?;

        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        if pending.commands.len() >= MAX_PENDING_MESSAGES {
            return Err(RenderServerPortError::QueueFull);
        }
        pending.commands.push_back(command);
        Ok(())
    }
}

impl RenderServerEventPort for RenderServerInterface {
    type Error = RenderServerPortError;

    fn next_event(
        &mut self,
        session_id: &SessionId,
    ) -> Result<Option<ServerEventEnvelope>, Self::Error> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        let Some(index) = pending
            .events
            .iter()
            .position(|event| &event.session_id == session_id)
        else {
            return Ok(None);
        };
        Ok(pending.events.remove(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewport_protocol::{ClientCommand, ServerEvent, SessionCommand, SessionEvent};

    #[test]
    fn command_port_validates_and_bounds_messages() {
        let interface = RenderServerInterface::default();
        let valid = ClientCommandEnvelope::for_session(
            "request-1",
            SessionId::new("session-1"),
            1,
            ClientCommand::Session(SessionCommand::Ping {
                nonce: "diagnostic".to_owned(),
            }),
        );

        interface.submit(valid).unwrap();
        assert_eq!(interface.pending_command_count(), 1);
    }

    #[test]
    fn event_port_polls_only_the_requested_session() {
        let mut interface = RenderServerInterface::default();
        interface
            .pending
            .lock()
            .unwrap()
            .events
            .push_back(ServerEventEnvelope::new(
                SessionId::new("session-2"),
                1,
                ServerEvent::Session(SessionEvent::Closed {
                    reason: Some("test".to_owned()),
                }),
            ));

        assert!(
            interface
                .next_event(&SessionId::new("session-1"))
                .unwrap()
                .is_none()
        );
        assert!(
            interface
                .next_event(&SessionId::new("session-2"))
                .unwrap()
                .is_some()
        );
        assert_eq!(interface.pending_event_count(), 0);
    }
}
