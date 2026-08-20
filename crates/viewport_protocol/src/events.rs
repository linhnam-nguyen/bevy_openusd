//! Event families emitted by the authoritative render server.

use serde::{Deserialize, Serialize};

use crate::{AuthorizationPolicy, HandshakeEvent, ViewportEvent, ViewportReadModel};

/// Top-level server events. UI state changes arrive through the semantic
/// viewport event family and are never synthesized by a transport adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "family", content = "payload", rename_all = "snake_case")]
pub enum ServerEvent {
    Handshake(HandshakeEvent),
    Session(SessionEvent),
    Stream(StreamEvent),
    Viewport(ViewportEvent),
}

/// Session lifecycle and diagnostic events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum SessionEvent {
    Ready {
        snapshot_required: bool,
    },
    Snapshot {
        state: ViewportReadModel,
    },
    SnapshotChunk {
        snapshot_id: String,
        chunk_index: u32,
        chunk_count: u32,
        state: ViewportReadModel,
    },
    AuthorizationChanged {
        authorization: AuthorizationPolicy,
    },
    SemanticSyncStatus {
        status: crate::SemanticSyncStatus,
    },
    RuntimeManifest {
        manifest: crate::AuthorizedRuntimeManifest,
    },
    RuntimeManifestChunk {
        manifest_id: String,
        chunk_index: u32,
        chunk_count: u32,
        manifest: crate::AuthorizedRuntimeManifest,
    },
    RuntimeBlobChunk {
        blob_id: String,
        chunk_index: u32,
        chunk_count: u32,
        bytes: Vec<u8>,
    },
    RuntimeBlobRejected {
        reason: String,
    },
    Resumed {
        result: crate::ResumeResult,
    },
    Pong {
        nonce: String,
    },
    Closed {
        reason: Option<String>,
    },
    HandshakeRejected {
        reason: crate::HandshakeRejectionReason,
    },
}

/// Stream lifecycle and active configuration events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum StreamEvent {
    ConfigurationAccepted {
        metrics: crate::ViewportMetrics,
    },
    ConfigurationApplied {
        configuration: crate::ActiveStreamConfiguration,
    },
    ConfigurationRejected {
        reason: String,
    },
    Statistics {
        statistics: crate::StreamStatistics,
    },
}
