use std::path::Path;
use viewport_protocol::{AuthorizationPolicy, SemanticSyncOperation, SessionId};

pub(crate) const MAX_PENDING_MESSAGES: usize = 256;

/// Project activation request admitted by one authenticated controller
/// session. The command carries identity only; the render host resolves the
/// private repository locator through its Project application service.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectActivationRequest {
    pub session_id: SessionId,
    pub command: project_protocol::ProjectActivationCommand,
}

/// Result routed back to the session that requested Project activation.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectActivationResult {
    pub session_id: SessionId,
    pub reply: project_protocol::ProjectActivationReply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderServerPortError {
    QueueClosed,
    QueueFull,
    InvalidPayload,
    SessionClosed,
}

/// Server-enriched semantic-sync request queued between the transport and the
/// application runtime. The authorization policy comes from the established
/// server session, never from the client command payload.
#[derive(Debug, Clone, PartialEq)]
pub enum SemanticSyncRequestKind {
    Client(SemanticSyncOperation),
    AuthorizationChanged,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticSyncRequest {
    pub request_id: String,
    pub session_id: SessionId,
    pub client_name: String,
    pub authorization: AuthorizationPolicy,
    pub kind: SemanticSyncRequestKind,
}

pub(super) fn safe_display_name(value: &str) -> String {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("remote-stage")
        .to_owned()
}
