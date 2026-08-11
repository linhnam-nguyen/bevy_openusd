//! Bevy-side ownership boundary for the external WebRTC transport.
//!
//! GStreamer and Tokio stay outside the ECS. This plugin only reserves the
//! lifecycle state resource that later phases will update through bounded
//! messages; Phase 1 does not route product commands into Bevy.

use bevy::prelude::*;

use crate::viewport::api::{RenderServerInterface, SessionRegistry};

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

pub struct WebRtcTransportPlugin;

impl Plugin for WebRtcTransportPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WebRtcTransportState>()
            .init_resource::<RenderServerInterface>();
    }
}
