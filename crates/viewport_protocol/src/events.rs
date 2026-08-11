//! Event families emitted by the authoritative render server.

use serde::{Deserialize, Serialize};

use crate::{ViewportEvent, ViewportReadModel};

/// Top-level server events. UI state changes arrive through the semantic
/// viewport event family and are never synthesized by a transport adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "family", content = "payload", rename_all = "snake_case")]
pub enum ServerEvent {
    Session(SessionEvent),
    Stream(StreamEvent),
    Viewport(ViewportEvent),
}

/// Session lifecycle and diagnostic events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum SessionEvent {
    Ready { snapshot_required: bool },
    Snapshot { state: ViewportReadModel },
    Resumed { result: crate::ResumeResult },
    Pong { nonce: String },
    Closed { reason: Option<String> },
    HandshakeRejected { reason: crate::HandshakeRejectionReason },
}

/// Stream lifecycle and active configuration events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum StreamEvent {
    ConfigurationAccepted { metrics: crate::ViewportMetrics },
    ConfigurationApplied { configuration: crate::ActiveStreamConfiguration },
    ConfigurationRejected { reason: String },
    Statistics { statistics: crate::StreamStatistics },
}
