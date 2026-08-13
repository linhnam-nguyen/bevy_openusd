//! Capability negotiation types used by the application handshake.

use serde::{Deserialize, Serialize};

use crate::{CodecId, StreamLimits};

/// Command families a peer can understand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandFamily {
    Session,
    Stream,
    Input,
    Viewport,
}

/// Input features supported by a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputCapabilities {
    pub pointer_motion: bool,
    pub wheel: bool,
    pub keyboard: bool,
    pub pointer_capture: bool,
}

impl Default for InputCapabilities {
    fn default() -> Self {
        Self {
            pointer_motion: true,
            wheel: true,
            keyboard: true,
            pointer_capture: true,
        }
    }
}

/// Client-side capabilities advertised during the handshake.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientCapabilities {
    pub protocol_versions: Vec<u16>,
    pub command_families: Vec<CommandFamily>,
    pub codecs: Vec<CodecId>,
    pub input: InputCapabilities,
    pub supports_dynamic_resolution: bool,
}

impl Default for ClientCapabilities {
    fn default() -> Self {
        Self {
            protocol_versions: vec![crate::PROTOCOL_VERSION],
            command_families: vec![
                CommandFamily::Session,
                CommandFamily::Stream,
                CommandFamily::Input,
                CommandFamily::Viewport,
            ],
            codecs: vec![CodecId::H264],
            input: InputCapabilities::default(),
            supports_dynamic_resolution: true,
        }
    }
}

/// Render-server capabilities and limits advertised to a client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerCapabilities {
    pub protocol_versions: Vec<u16>,
    pub command_families: Vec<CommandFamily>,
    pub codecs: Vec<CodecId>,
    pub input: InputCapabilities,
    pub stream_limits: StreamLimits,
    pub supports_dynamic_resolution: bool,
}

impl Default for ServerCapabilities {
    fn default() -> Self {
        Self {
            protocol_versions: vec![crate::PROTOCOL_VERSION],
            command_families: vec![
                CommandFamily::Session,
                CommandFamily::Stream,
                CommandFamily::Input,
                CommandFamily::Viewport,
            ],
            codecs: vec![CodecId::H264],
            input: InputCapabilities::default(),
            stream_limits: StreamLimits::default(),
            supports_dynamic_resolution: true,
        }
    }
}

impl ServerCapabilities {
    /// Returns the server capabilities for the codec selected at launch.
    pub fn for_codec(codec: CodecId) -> Self {
        Self {
            codecs: vec![codec],
            ..Self::default()
        }
    }
}
