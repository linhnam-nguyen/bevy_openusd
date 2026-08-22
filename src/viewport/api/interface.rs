//! Bevy-facing re-export of the transport-neutral render-server application bus.
//!
//! The concrete queue lives in `viewport_streaming` so the WebRTC library and
//! the Bevy binary share the same dependency direction. This module keeps the
//! existing Bevy API ownership narrow and transport-agnostic.

use bevy::prelude::Resource;

/// Bevy-owned resource wrapper around the transport crate's thread-safe bus.
#[derive(Debug, Clone, Default, Resource)]
pub(crate) struct RenderServerInterface(viewport_streaming::RenderServerInterface);

impl RenderServerInterface {
    pub(crate) fn shared(&self) -> viewport_streaming::RenderServerInterface {
        self.0.clone()
    }

    pub(crate) fn pop_viewport_command(
        &self,
    ) -> Option<viewport_protocol::ViewportCommandEnvelope> {
        self.0.pop_viewport_command()
    }

    pub(crate) fn pop_input(&self) -> Option<viewport_protocol::InputCommand> {
        self.0.pop_input()
    }

    pub(crate) fn take_latest_pointer_motion(&self) -> Option<viewport_protocol::PointerMotion> {
        self.0.take_latest_pointer_motion()
    }

    pub(crate) fn take_input_reset(&self) -> bool {
        self.0.take_input_reset()
    }

    pub(crate) fn take_stream_configuration(&self) -> Option<viewport_protocol::ViewportMetrics> {
        self.0.take_stream_configuration()
    }

    pub(crate) fn submit_viewport_command(
        &self,
        command: viewport_protocol::ViewportCommandEnvelope,
    ) -> Result<(), viewport_streaming::RenderServerPortError> {
        self.0.submit_viewport_command(command)
    }

    pub(crate) fn submit_input(
        &self,
        command: viewport_protocol::InputCommand,
    ) -> Result<(), viewport_streaming::RenderServerPortError> {
        self.0.submit_input(command)
    }

    pub(crate) fn submit_pointer_motion(
        &self,
        motion: viewport_protocol::PointerMotion,
    ) -> Result<(), viewport_streaming::RenderServerPortError> {
        self.0
            .submit_input(viewport_protocol::InputCommand::PointerMotion(motion))
    }

    pub(crate) fn publish_viewport_event(
        &self,
        event: viewport_protocol::ViewportEventEnvelope,
    ) -> Result<(), viewport_streaming::RenderServerPortError> {
        self.0.publish_viewport_event(event)
    }

    pub(crate) fn pop_viewport_event(&self) -> Option<viewport_protocol::ViewportEventEnvelope> {
        self.0.pop_viewport_event()
    }
}
