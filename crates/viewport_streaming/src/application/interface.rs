use std::sync::{Arc, Mutex};

use viewport_protocol::{
    ClientCommandEnvelope, InputCommand, PointerMotion, ViewportCommandEnvelope, ViewportEvent,
    ViewportEventEnvelope, ViewportMetrics, ViewportReadModel,
};

use super::state::PendingMessages;
use super::types::{MAX_PENDING_MESSAGES, RenderServerPortError, safe_display_name};

/// Transport-neutral application boundary shared across the ECS and WebRTC
/// threads. It contains no Bevy, GStreamer, Tokio, or DOM objects.
#[derive(Debug, Clone, Default)]
pub struct RenderServerInterface {
    pub(super) pending: Arc<Mutex<PendingMessages>>,
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

    /// Assigns a server-monotonic generation and queues the newest validated viewport request.
    pub fn submit_stream_configuration(
        &self,
        mut metrics: ViewportMetrics,
    ) -> Result<ViewportMetrics, RenderServerPortError> {
        metrics
            .validate()
            .map_err(|_| RenderServerPortError::InvalidPayload)?;
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        let next_generation = pending.latest_stream_generation.saturating_add(1);
        metrics.generation = metrics.generation.max(next_generation);
        pending.latest_stream_generation = metrics.generation;
        pending.pending_stream_configuration = Some(metrics.clone());
        Ok(metrics)
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

    /// Clears all remote input when a peer/channel disappears.
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

    /// Returns the newest state and discards historical events.
    pub fn take_latest_snapshot(&self, fallback: ViewportReadModel) -> ViewportReadModel {
        let mut pending = self
            .pending
            .lock()
            .expect("render-server interface queue is not poisoned");
        pending.viewport_events.clear();
        pending.latest_snapshot.clone().unwrap_or(fallback)
    }
}
