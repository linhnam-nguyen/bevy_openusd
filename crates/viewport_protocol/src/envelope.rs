//! Session, correlation, and sequence metadata shared by protocol envelopes.

use serde::{Deserialize, Serialize};
use std::{error::Error, fmt};

use crate::{ClientCommand, PROTOCOL_VERSION, ServerEvent};

/// A client-generated identifier used to correlate a command with its result.
pub type RequestId = String;

/// An identifier for the command or event that caused another message.
pub type CausationId = String;

/// A monotonically increasing sequence number within one direction/session.
pub type SequenceNumber = u64;

/// Stable identity for a viewport session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        if self.0.trim().is_empty() {
            return Err(ProtocolValidationError::EmptyField { field: "session_id" });
        }
        Ok(())
    }
}

/// Validation failures exposed by the transport-neutral contract.
#[derive(Debug, Clone, PartialEq)]
pub enum ProtocolValidationError {
    UnsupportedProtocolVersion { received: u16, expected: u16 },
    EmptyField { field: &'static str },
    InvalidDimension { field: &'static str, value: u32 },
    DimensionOutOfRange { field: &'static str, value: u32, maximum: u32 },
    PixelCountOutOfRange { width: u32, height: u32, maximum: u64 },
    InvalidDevicePixelRatio { value: f32 },
    InvalidFrameRate { value: u32 },
    OddEncodedDimension { field: &'static str, value: u32 },
    InvalidInput { field: &'static str },
    InvalidSequence,
}

impl fmt::Display for ProtocolValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedProtocolVersion { received, expected } => write!(
                formatter,
                "unsupported protocol version {received}; expected {expected}"
            ),
            Self::EmptyField { field } => write!(formatter, "{field} must not be empty"),
            Self::InvalidDimension { field, value } => {
                write!(formatter, "{field} must be greater than zero; got {value}")
            }
            Self::DimensionOutOfRange {
                field,
                value,
                maximum,
            } => write!(formatter, "{field} must be at most {maximum}; got {value}"),
            Self::PixelCountOutOfRange {
                width,
                height,
                maximum,
            } => write!(
                formatter,
                "requested pixel count {width}x{height} exceeds maximum {maximum}"
            ),
            Self::InvalidDevicePixelRatio { value } => {
                write!(formatter, "device pixel ratio must be finite and in range; got {value}")
            }
            Self::InvalidFrameRate { value } => {
                write!(formatter, "frame rate must be between 1 and 240; got {value}")
            }
            Self::OddEncodedDimension { field, value } => {
                write!(formatter, "{field} must be even for the encoded stream; got {value}")
            }
            Self::InvalidInput { field } => write!(formatter, "invalid viewport input: {field}"),
            Self::InvalidSequence => write!(formatter, "sequence must be greater than zero"),
        }
    }
}

impl Error for ProtocolValidationError {}

/// Validates a wire/API version against this crate's supported version.
pub fn validate_protocol_version(version: u16) -> Result<(), ProtocolValidationError> {
    if version == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(ProtocolValidationError::UnsupportedProtocolVersion {
            received: version,
            expected: PROTOCOL_VERSION,
        })
    }
}

/// Versioned command envelope for the transport-neutral client boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientCommandEnvelope {
    pub protocol_version: u16,
    pub request_id: RequestId,
    pub session_id: Option<SessionId>,
    pub sequence: SequenceNumber,
    pub causation_id: Option<CausationId>,
    pub command: ClientCommand,
}

impl ClientCommandEnvelope {
    /// Creates a command with deterministic metadata for a new client sequence.
    pub fn new(
        request_id: impl Into<RequestId>,
        sequence: SequenceNumber,
        command: ClientCommand,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            session_id: None,
            sequence,
            causation_id: None,
            command,
        }
    }

    pub fn for_session(
        request_id: impl Into<RequestId>,
        session_id: SessionId,
        sequence: SequenceNumber,
        command: ClientCommand,
    ) -> Self {
        let mut envelope = Self::new(request_id, sequence, command);
        envelope.session_id = Some(session_id);
        envelope
    }

    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_protocol_version(self.protocol_version)?;
        if self.request_id.trim().is_empty() {
            return Err(ProtocolValidationError::EmptyField { field: "request_id" });
        }
        if self.sequence == 0 {
            return Err(ProtocolValidationError::InvalidSequence);
        }
        if let Some(session_id) = &self.session_id {
            session_id.validate()?;
        }
        self.command.validate()
    }
}

/// Versioned event envelope published by the authoritative render server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerEventEnvelope {
    pub protocol_version: u16,
    pub session_id: SessionId,
    pub sequence: SequenceNumber,
    pub request_id: Option<RequestId>,
    pub causation_id: Option<CausationId>,
    pub event: ServerEvent,
}

impl ServerEventEnvelope {
    pub fn new(session_id: SessionId, sequence: SequenceNumber, event: ServerEvent) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            session_id,
            sequence,
            request_id: None,
            causation_id: None,
            event,
        }
    }

    pub fn for_request(
        session_id: SessionId,
        sequence: SequenceNumber,
        request_id: impl Into<RequestId>,
        event: ServerEvent,
    ) -> Self {
        let mut envelope = Self::new(session_id, sequence, event);
        envelope.request_id = Some(request_id.into());
        envelope
    }

    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_protocol_version(self.protocol_version)?;
        self.session_id.validate()?;
        if self.sequence == 0 {
            return Err(ProtocolValidationError::InvalidSequence);
        }
        if self
            .request_id
            .as_ref()
            .is_some_and(|request_id| request_id.trim().is_empty())
        {
            return Err(ProtocolValidationError::EmptyField { field: "request_id" });
        }
        Ok(())
    }
}
