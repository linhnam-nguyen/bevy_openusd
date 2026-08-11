//! Command families for the remote viewport boundary.

use serde::{Deserialize, Serialize};

use crate::{
    ButtonState, FocusState, InputModifiers, KeyboardInput, PointerMotion,
    ClientHello, ProtocolValidationError, ReleaseAllInput, ViewportCommand, ViewportMetrics,
};

/// Commands accepted from a client. The semantic viewport commands remain in
/// [`ViewportCommand`] so existing stdio clients can migrate incrementally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "family", content = "payload", rename_all = "snake_case")]
pub enum ClientCommand {
    Handshake(ClientHello),
    Session(SessionCommand),
    Stream(StreamCommand),
    Input(InputCommand),
    Viewport(ViewportCommand),
}

impl ClientCommand {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        match self {
            Self::Handshake(hello) => hello.validate(),
            Self::Stream(StreamCommand::ConfigureViewport { metrics }) => metrics.validate(),
            Self::Session(_) | Self::Stream(_) | Self::Input(_) | Self::Viewport(_) => Ok(()),
        }
    }
}

/// Session lifecycle commands. Transport connection setup is intentionally
/// separate from these application-level messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum SessionCommand {
    RequestSnapshot,
    Resume { request: crate::ResumeRequest },
    Close { reason: Option<String> },
    Ping { nonce: String },
}

/// Stream configuration and diagnostic commands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum StreamCommand {
    ConfigureViewport { metrics: ViewportMetrics },
    RequestKeyframe,
}

/// Input commands are split from semantic viewport commands so high-frequency
/// motion can use a separate low-latency transport channel later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum InputCommand {
    PointerMotion(PointerMotion),
    ButtonState(ButtonState),
    Keyboard(KeyboardInput),
    FocusChanged(FocusState),
    ReleaseAll(ReleaseAllInput),
    SetModifiers(InputModifiers),
}
