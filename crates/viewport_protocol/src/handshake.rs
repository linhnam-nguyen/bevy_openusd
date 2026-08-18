//! Application handshake and reconnect/resume messages.

use serde::{Deserialize, Serialize};

use crate::{
    AuthorizationPolicy, ClientCapabilities, PROTOCOL_VERSION, ProtocolValidationError,
    ServerCapabilities, SessionId, ViewportMetrics,
};

/// Role negotiated for a connected viewport client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionRole {
    Controller,
    Observer,
}

/// First application message sent after the reliable control channel opens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientHello {
    pub protocol_version: u16,
    pub client_id: String,
    pub requested_session_id: Option<SessionId>,
    pub requested_role: SessionRole,
    pub capabilities: ClientCapabilities,
    pub initial_viewport: ViewportMetrics,
    pub resume: Option<ResumeRequest>,
}

impl ClientHello {
    pub fn new(client_id: impl Into<String>, capabilities: ClientCapabilities) -> Self {
        Self::with_viewport(client_id, capabilities, ViewportMetrics::default())
    }

    pub fn with_viewport(
        client_id: impl Into<String>,
        capabilities: ClientCapabilities,
        initial_viewport: ViewportMetrics,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            client_id: client_id.into(),
            requested_session_id: None,
            requested_role: SessionRole::Controller,
            capabilities,
            initial_viewport,
            resume: None,
        }
    }

    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        crate::envelope::validate_protocol_version(self.protocol_version)?;
        if self.client_id.trim().is_empty() {
            return Err(ProtocolValidationError::EmptyField { field: "client_id" });
        }
        if let Some(session_id) = &self.requested_session_id {
            session_id.validate()?;
        }
        self.initial_viewport.validate()?;
        Ok(())
    }
}

/// Server response that establishes the active application session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerHello {
    pub protocol_version: u16,
    pub session_id: SessionId,
    pub role: SessionRole,
    pub capabilities: ServerCapabilities,
    #[serde(default, skip_serializing_if = "AuthorizationPolicy::is_default")]
    pub authorization: AuthorizationPolicy,
    pub resumed: bool,
}

impl ServerHello {
    pub fn new(session_id: SessionId, role: SessionRole, capabilities: ServerCapabilities) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            session_id,
            role,
            capabilities,
            authorization: AuthorizationPolicy::default(),
            resumed: false,
        }
    }

    /// Construct a server hello with an explicit session authorization policy.
    pub fn with_authorization(
        session_id: SessionId,
        role: SessionRole,
        capabilities: ServerCapabilities,
        authorization: AuthorizationPolicy,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            session_id,
            role,
            capabilities,
            authorization,
            resumed: false,
        }
    }

    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        crate::envelope::validate_protocol_version(self.protocol_version)?;
        self.session_id.validate()?;
        self.authorization
            .validate()
            .map_err(|_| ProtocolValidationError::InvalidInput {
                field: "authorization",
            })
    }
}

/// Handshake messages reserved for the application/session coordinator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum HandshakeEvent {
    ClientHello(ClientHello),
    ServerHello(ServerHello),
    Ready { session_id: SessionId },
    Rejected { reason: HandshakeRejectionReason },
}

/// A reconnect request carrying the last authoritative event observed by a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeRequest {
    pub session_id: SessionId,
    pub last_server_sequence: u64,
}

/// Server result for a resume attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeResult {
    pub accepted: bool,
    pub session_id: SessionId,
    pub next_server_sequence: u64,
    pub reason: Option<String>,
}

/// Explicit handshake failures that the UI can present without parsing text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandshakeRejectionReason {
    UnsupportedProtocolVersion,
    InvalidClientIdentity,
    UnsupportedCapabilities,
    ControllerAlreadyAssigned,
    ResumeUnavailable,
}
