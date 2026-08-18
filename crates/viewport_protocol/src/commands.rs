//! Command families for the remote viewport boundary.

use serde::{Deserialize, Serialize};

use crate::{
    ButtonState, ClientHello, FocusState, InputModifiers, KeyboardInput, PointerMotion,
    ProtocolValidationError, ReleaseAllInput, ViewportCommand, ViewportMetrics,
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
            Self::Input(input) => input.validate(),
            Self::Session(command) => command.validate(),
            Self::Stream(_) => Ok(()),
            Self::Viewport(command) => command.validate(),
        }
    }
}

/// Session lifecycle commands. Transport connection setup is intentionally
/// separate from these application-level messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum SessionCommand {
    RequestSnapshot,
    RequestRuntimeManifest,
    RequestRuntimeBlob { blob_id: String },
    Resume { request: crate::ResumeRequest },
    Close { reason: Option<String> },
    Ping { nonce: String },
}

impl SessionCommand {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        if let Self::RequestRuntimeBlob { blob_id } = self {
            crate::validate_runtime_blob_id(blob_id).map_err(|_| {
                ProtocolValidationError::InvalidInput {
                    field: "runtime.blob_id",
                }
            })?;
        }
        Ok(())
    }
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

const MAX_INPUT_DELTA_CSS_PIXELS: f32 = 4096.0;
const MAX_INPUT_VIEWPORT_CSS_PIXELS: f32 = 100_000.0;

impl InputCommand {
    /// Validates browser/native input before it crosses into the renderer.
    /// Motion is intentionally bounded to avoid a malformed packet creating
    /// an unbounded camera jump or a NaN transform.
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        match self {
            Self::PointerMotion(motion) => {
                validate_sequence(motion.sequence)?;
                validate_bounded_finite(
                    "pointer.dx_css_pixels",
                    motion.dx_css_pixels,
                    MAX_INPUT_DELTA_CSS_PIXELS,
                )?;
                validate_bounded_finite(
                    "pointer.dy_css_pixels",
                    motion.dy_css_pixels,
                    MAX_INPUT_DELTA_CSS_PIXELS,
                )?;
                validate_bounded_finite(
                    "pointer.wheel_x",
                    motion.wheel_x,
                    MAX_INPUT_DELTA_CSS_PIXELS,
                )?;
                validate_bounded_finite(
                    "pointer.wheel_y",
                    motion.wheel_y,
                    MAX_INPUT_DELTA_CSS_PIXELS,
                )?;
                if !motion.viewport_css_width.is_finite()
                    || !motion.viewport_css_height.is_finite()
                    || motion.viewport_css_width <= 0.0
                    || motion.viewport_css_height <= 0.0
                    || motion.viewport_css_width > MAX_INPUT_VIEWPORT_CSS_PIXELS
                    || motion.viewport_css_height > MAX_INPUT_VIEWPORT_CSS_PIXELS
                {
                    return Err(ProtocolValidationError::InvalidInput {
                        field: "pointer.viewport_css_size",
                    });
                }
            }
            Self::ButtonState(state) => validate_sequence(state.sequence)?,
            Self::Keyboard(keyboard) => {
                validate_sequence(keyboard.sequence)?;
                if keyboard.code.trim().is_empty() {
                    return Err(ProtocolValidationError::EmptyField {
                        field: "keyboard.code",
                    });
                }
                if keyboard.code.len() > 64 {
                    return Err(ProtocolValidationError::InvalidInput {
                        field: "keyboard.code.length",
                    });
                }
            }
            Self::FocusChanged(focus) => validate_sequence(focus.sequence)?,
            Self::ReleaseAll(release) => validate_sequence(release.sequence)?,
            Self::SetModifiers(_) => {}
        }
        Ok(())
    }
}

fn validate_sequence(sequence: u64) -> Result<(), ProtocolValidationError> {
    if sequence == 0 {
        Err(ProtocolValidationError::InvalidSequence)
    } else {
        Ok(())
    }
}

fn validate_finite(field: &'static str, value: f32) -> Result<(), ProtocolValidationError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(ProtocolValidationError::InvalidInput { field })
    }
}

fn validate_bounded_finite(
    field: &'static str,
    value: f32,
    maximum_absolute_value: f32,
) -> Result<(), ProtocolValidationError> {
    validate_finite(field, value)?;
    if value.abs() > maximum_absolute_value {
        return Err(ProtocolValidationError::InvalidInput { field });
    }
    Ok(())
}
