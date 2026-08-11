//! Shared application bus between the Bevy viewport and WebRTC sessions.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use viewport_protocol::{
    ClientCommandEnvelope, ViewportCommandEnvelope, ViewportEvent, ViewportEventEnvelope,
    ViewportReadModel,
};

const MAX_PENDING_MESSAGES: usize = 256;

#[derive(Debug, Default)]
struct PendingMessages {
    commands: VecDeque<ClientCommandEnvelope>,
    viewport_commands: VecDeque<ViewportCommandEnvelope>,
    viewport_events: VecDeque<ViewportEventEnvelope>,
    latest_snapshot: Option<ViewportReadModel>,
}

/// Transport-neutral application boundary shared across the ECS and WebRTC
/// threads. It contains no Bevy, GStreamer, Tokio, or DOM objects.
#[derive(Debug, Clone, Default)]
pub struct RenderServerInterface {
    pending: Arc<Mutex<PendingMessages>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderServerPortError {
    QueueClosed,
    QueueFull,
    InvalidPayload,
}

impl RenderServerInterface {
    pub fn pending_command_count(&self) -> usize {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .commands
            .len()
    }

    pub fn pending_event_count(&self) -> usize {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .viewport_events
            .len()
    }

    pub fn submit(&self, command: ClientCommandEnvelope) -> Result<(), RenderServerPortError> {
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

    pub fn pop_command(&self) -> Option<ClientCommandEnvelope> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .commands
            .pop_front()
    }

    pub fn submit_viewport_command(
        &self,
        command: ViewportCommandEnvelope,
    ) -> Result<(), RenderServerPortError> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        if pending.viewport_commands.len() >= MAX_PENDING_MESSAGES {
            return Err(RenderServerPortError::QueueFull);
        }
        pending.viewport_commands.push_back(command);
        Ok(())
    }

    pub fn pop_viewport_command(&self) -> Option<ViewportCommandEnvelope> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .viewport_commands
            .pop_front()
    }

    pub fn publish_viewport_event(
        &self,
        mut event: ViewportEventEnvelope,
    ) -> Result<(), RenderServerPortError> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        if pending.viewport_events.len() >= MAX_PENDING_MESSAGES {
            return Err(RenderServerPortError::QueueFull);
        }

        if let ViewportEvent::Snapshot { state } = &mut event.event {
            state.stage.display_name = safe_display_name(&state.stage.display_name);
            pending.latest_snapshot = Some(state.clone());
        }
        pending.viewport_events.push_back(event);
        Ok(())
    }

    pub fn pop_viewport_event(&self) -> Option<ViewportEventEnvelope> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .viewport_events
            .pop_front()
    }

    /// Returns the newest state and discards historical events. A newly
    /// connected session receives the current snapshot first; replaying
    /// pre-handshake lifecycle events could regress it to an older state.
    pub fn take_latest_snapshot(&self, fallback: ViewportReadModel) -> ViewportReadModel {
        let mut pending = self
            .pending
            .lock()
            .expect("render-server interface queue is not poisoned");
        pending.viewport_events.clear();
        pending.latest_snapshot.clone().unwrap_or(fallback)
    }
}

fn safe_display_name(value: &str) -> String {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("remote-stage")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_display_name_is_reduced_to_a_basename() {
        let interface = RenderServerInterface::default();
        interface
            .publish_viewport_event(ViewportEventEnvelope::new(
                None,
                ViewportEvent::Snapshot {
                    state: ViewportReadModel::unloaded("/private/stages/Kitchen_set.usdz"),
                },
            ))
            .unwrap();

        assert_eq!(
            interface
                .take_latest_snapshot(ViewportReadModel::unloaded("fallback.usda"))
                .stage
                .display_name,
            "Kitchen_set.usdz"
        );
    }

    #[test]
    fn event_history_is_cleared_when_a_session_takes_a_snapshot() {
        let interface = RenderServerInterface::default();
        let snapshot = ViewportReadModel::unloaded("stage.usda");
        interface
            .publish_viewport_event(ViewportEventEnvelope::new(
                None,
                ViewportEvent::Snapshot {
                    state: snapshot.clone(),
                },
            ))
            .unwrap();

        assert_eq!(interface.pending_event_count(), 1);
        assert_eq!(interface.take_latest_snapshot(snapshot.clone()), snapshot);
        assert_eq!(interface.pending_event_count(), 0);
    }
}
