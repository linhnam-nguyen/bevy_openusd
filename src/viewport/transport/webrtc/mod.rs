//! Bevy-side ownership boundary for the external WebRTC transport.
//!
//! GStreamer and Tokio stay outside the ECS. Commands and authoritative events
//! cross the boundary through the shared application interface.

use bevy::prelude::*;
use std::path::Path;
use viewport_protocol::{
    InputCommand, SessionId, SessionRole, ViewportEvent, ViewportEventEnvelope,
};

use crate::viewport::api::{
    RenderServerInterface, SessionRegistry, ViewportBridgeSet, ViewportCommandInbox,
    ViewportEventOutbox,
};
use crate::viewport::camera::ArcballCameraSet;
use crate::viewport::input::{apply_remote_navigation_input, sync_headless_gizmo_input};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChannelState {
    #[default]
    Closed,
    Connecting,
    Open,
}

#[allow(dead_code)]
#[derive(Debug, Resource, Default)]
pub struct WebRtcTransportState {
    pub control: ChannelState,
    pub input: ChannelState,
    pub sessions: SessionRegistry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthenticatedInputError {
    NotController,
    QueueRejected,
}

impl WebRtcTransportState {
    /// Applies the same in-memory admission snapshot used after a WebRTC
    /// session handshake before allowing input onto the renderer bus. The
    /// benchmark uses this adapter to exercise the production input consumer;
    /// database/network authorization is intentionally not performed here.
    pub(crate) fn submit_authenticated_input(
        &self,
        session_id: &SessionId,
        interface: &RenderServerInterface,
        command: InputCommand,
    ) -> Result<(), AuthenticatedInputError> {
        if self.sessions.role(session_id) != Some(SessionRole::Controller) {
            return Err(AuthenticatedInputError::NotController);
        }
        interface
            .submit_input(command)
            .map_err(|_| AuthenticatedInputError::QueueRejected)
    }
}

pub struct WebRtcTransportPlugin;

impl Plugin for WebRtcTransportPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WebRtcTransportState>()
            .init_resource::<RenderServerInterface>()
            .add_systems(
                Update,
                apply_remote_navigation_input
                    .after(ArcballCameraSet::PrepareInput)
                    .before(ArcballCameraSet::ApplyInput),
            )
            .add_systems(
                Update,
                sync_headless_gizmo_input.after(apply_remote_navigation_input),
            )
            .add_systems(
                Update,
                drain_remote_commands.before(ViewportBridgeSet::ApplyCommands),
            )
            .add_systems(
                Update,
                publish_authoritative_events.after(ViewportBridgeSet::ReduceEvents),
            );
    }
}

pub(crate) fn drain_remote_commands(
    interface: Res<RenderServerInterface>,
    mut inbox: ResMut<ViewportCommandInbox>,
    mut counters: Option<ResMut<crate::viewport::diagnostics::performance::RendererCounters>>,
) {
    while let Some(command) = interface.pop_viewport_command() {
        if let Some(ref mut c) = counters {
            c.remote_commands_drained += 1;
            if c.first_remote_command_received_frame.is_none() {
                c.first_remote_command_received_frame = Some(c.frame_count);
            }
        }
        inbox.push(command);
    }
}

pub(crate) fn publish_authoritative_events(
    interface: Res<RenderServerInterface>,
    mut outbox: ResMut<ViewportEventOutbox>,
    mut counters: Option<ResMut<crate::viewport::diagnostics::performance::RendererCounters>>,
) {
    while let Some(event) = outbox.pop() {
        let event = sanitize_event(event);
        if let Err(error) = interface.publish_viewport_event(event.clone()) {
            bevy::log::warn!(
                "[viewport-webrtc] authoritative event queue rejected an event: {error:?}"
            );
            outbox.push_front(event);
            break;
        } else if let Some(ref mut c) = counters {
            c.authoritative_events_published += 1;
            if c.first_authoritative_event_published_frame.is_none() {
                c.first_authoritative_event_published_frame = Some(c.frame_count);
            }
        }
    }
}

fn sanitize_event(mut event: ViewportEventEnvelope) -> ViewportEventEnvelope {
    if let ViewportEvent::Snapshot { state } = &mut event.event {
        state.stage.display_name = Path::new(&state.stage.display_name)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("remote-stage")
            .to_owned();
    }
    event
}
