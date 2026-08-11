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

    pub(crate) fn publish_viewport_event(
        &self,
        event: viewport_protocol::ViewportEventEnvelope,
    ) -> Result<(), viewport_streaming::RenderServerPortError> {
        self.0.publish_viewport_event(event)
    }
}
