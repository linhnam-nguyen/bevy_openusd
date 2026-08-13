//! Shared application bus between the Bevy viewport and WebRTC sessions.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use viewport_protocol::{
    ClientCommandEnvelope, InputCommand, PointerMotion, ViewportCommandEnvelope, ViewportEvent,
    ViewportEventEnvelope, ViewportMetrics, ViewportReadModel,
};

const MAX_PENDING_MESSAGES: usize = 256;

#[derive(Debug, Default)]
struct PendingMessages {
    commands: VecDeque<ClientCommandEnvelope>,
    viewport_commands: VecDeque<ViewportCommandEnvelope>,
    input_commands: VecDeque<InputCommand>,
    latest_pointer_motion: Option<PointerMotion>,
    last_pointer_sequence: u64,
    input_reset: bool,
    viewport_events: VecDeque<ViewportEventEnvelope>,
    latest_snapshot: Option<ViewportReadModel>,
    pending_stream_configuration: Option<ViewportMetrics>,
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

    pub fn pending_input_count(&self) -> usize {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .input_commands
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
        command
            .validate()
            .map_err(|_| RenderServerPortError::InvalidPayload)?;
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

    /// Queues reliable input state and keeps only the newest motion packet.
    /// The latter is deliberately replaceable so pointer traffic cannot
    /// block semantic commands or make the camera process stale deltas.
    pub fn submit_input(&self, command: InputCommand) -> Result<(), RenderServerPortError> {
        command
            .validate()
            .map_err(|_| RenderServerPortError::InvalidPayload)?;

        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        if let InputCommand::PointerMotion(motion) = command {
            if motion.sequence <= pending.last_pointer_sequence {
                return Ok(());
            }
            pending.last_pointer_sequence = motion.sequence;
            pending.latest_pointer_motion = Some(motion);
            return Ok(());
        }
        if pending.input_commands.len() >= MAX_PENDING_MESSAGES {
            return Err(RenderServerPortError::QueueFull);
        }
        pending.input_commands.push_back(command);
        Ok(())
    }

    pub fn pop_input(&self) -> Option<InputCommand> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .input_commands
            .pop_front()
    }

    /// Queues the newest validated initial viewport request for the Bevy main
    /// thread. The transport callback never mutates `Assets<Image>` directly.
    pub fn submit_stream_configuration(
        &self,
        metrics: ViewportMetrics,
    ) -> Result<(), RenderServerPortError> {
        metrics
            .validate()
            .map_err(|_| RenderServerPortError::InvalidPayload)?;
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        let replace = pending
            .pending_stream_configuration
            .as_ref()
            .is_none_or(|current| metrics.generation >= current.generation);
        if replace {
            pending.pending_stream_configuration = Some(metrics);
        }
        Ok(())
    }

    pub fn take_stream_configuration(&self) -> Option<ViewportMetrics> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .pending_stream_configuration
            .take()
    }

    pub fn take_latest_pointer_motion(&self) -> Option<PointerMotion> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .latest_pointer_motion
            .take()
    }

    /// Clears all remote input when a peer/channel disappears. The Bevy
    /// adapter observes the reset marker on its next update tick.
    pub fn clear_remote_input(&self) {
        let mut pending = self
            .pending
            .lock()
            .expect("render-server interface queue is not poisoned");
        pending.input_commands.clear();
        pending.latest_pointer_motion = None;
        pending.last_pointer_sequence = 0;
        pending.input_reset = true;
    }

    pub fn take_input_reset(&self) -> bool {
        let mut pending = self
            .pending
            .lock()
            .expect("render-server interface queue is not poisoned");
        let reset = pending.input_reset;
        pending.input_reset = false;
        reset
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

    pub fn requeue_viewport_event_front(
        &self,
        event: ViewportEventEnvelope,
    ) -> Result<(), RenderServerPortError> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        if pending.viewport_events.len() >= MAX_PENDING_MESSAGES {
            return Err(RenderServerPortError::QueueFull);
        }
        pending.viewport_events.push_front(event);
        Ok(())
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
    use viewport_protocol::{InputCommand, PointerMotion, ViewportCommand};

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

    #[test]
    fn viewport_command_queue_rejects_invalid_correlation_metadata() {
        let interface = RenderServerInterface::default();
        let command = ViewportCommandEnvelope::new("", ViewportCommand::RequestSnapshot);

        assert_eq!(
            interface.submit_viewport_command(command),
            Err(RenderServerPortError::InvalidPayload)
        );
        assert_eq!(interface.pending_command_count(), 0);
    }

    #[test]
    fn an_event_can_be_put_back_after_a_failed_transport_send() {
        let interface = RenderServerInterface::default();
        let event = ViewportEventEnvelope::new(
            Some("request-1".to_owned()),
            ViewportEvent::PhysicsChanged { running: true },
        );
        interface
            .publish_viewport_event(event.clone())
            .expect("event should enter the bounded queue");
        let removed = interface
            .pop_viewport_event()
            .expect("event should be available for transport");
        interface
            .requeue_viewport_event_front(removed)
            .expect("failed sends must be recoverable");

        assert_eq!(interface.pop_viewport_event(), Some(event));
    }

    #[test]
    fn pointer_motion_keeps_only_the_newest_valid_packet() {
        let interface = RenderServerInterface::default();
        let motion = |sequence| PointerMotion {
            sequence,
            dx_css_pixels: sequence as f32,
            dy_css_pixels: 0.0,
            wheel_x: 0.0,
            wheel_y: 0.0,
            viewport_css_width: 800.0,
            viewport_css_height: 600.0,
            stream_generation: 0,
        };

        interface
            .submit_input(InputCommand::PointerMotion(motion(1)))
            .unwrap();
        interface
            .submit_input(InputCommand::PointerMotion(motion(2)))
            .unwrap();
        interface
            .submit_input(InputCommand::PointerMotion(motion(1)))
            .unwrap();

        assert_eq!(
            interface.take_latest_pointer_motion().unwrap().sequence,
            2
        );
        assert!(interface.take_latest_pointer_motion().is_none());
    }

    #[test]
    fn reliable_input_state_is_preserved_in_order() {
        let interface = RenderServerInterface::default();
        interface
            .submit_input(InputCommand::ReleaseAll(
                viewport_protocol::ReleaseAllInput { sequence: 1 },
            ))
            .unwrap();
        assert!(matches!(
            interface.pop_input(),
            Some(InputCommand::ReleaseAll(_))
        ));
    }
}
