//! Transport-neutral semantic projection synchronization status.

use serde::{Deserialize, Serialize};

/// Client-requested semantic synchronization operation.
///
/// The operation intentionally carries no authorization policy, remote URL,
/// database name, or credential. The server derives authorization from the
/// established session and keeps all deployment details private.
///
/// # Lease and Token Rotation Policy (Milestone 24 / R6)
/// - **Policy Change Rotation**: An authorization policy change revokes the
///   per-session database lease, drops client credentials, and marks the session
///   `Stale`. Reconnecting/reprovisioning requires an explicit fresh lease under
///   the new server-approved policy.
/// - **Disconnect Rotation**: Disconnecting or submitting `Close` revokes the
///   database lease and destroys credentials. Reconnecting begins a new session
///   lifecycle that receives a fresh lease.
/// - **Token Lifetime & Refresh**: Token expiration is configured server-side
///   (e.g., `TURSO_CLIENT_TOKEN_EXPIRATION`). Milestone 24 does not perform
///   in-place background token mutation or timer refreshes; token expiration
///   requires explicit reprovisioning. Clients never choose or negotiate raw
///   Turso token lifetime directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSyncOperation {
    Provision,
    Connect,
    PushSnapshot,
    PullProjection,
    Close,
}

/// Server-owned lifecycle phase for an authorized semantic projection.
///
/// Credentials, remote URLs, and SQL never appear in this type. It is safe to
/// expose the status through the viewport application protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSyncPhase {
    Disabled,
    Provisioning,
    Provisioned,
    Connecting,
    Ready,
    Pulling,
    Pushing,
    Stale,
    Failed,
    Closed,
}

/// The current synchronization read model for one viewport session.
///
/// # Lifecycle Semantics (Milestone 24 / R5)
/// - `SemanticSyncStatus` represents the **latest authoritative lifecycle state**
///   for a given `SessionId`.
/// - It is **NOT** an asynchronous completion receipt or result for a specific
///   `request_id X`.
/// - Individual request IDs are used at the transport layer for immediate command
///   validation and deduplication, while `SemanticSyncStatus` reflects the
///   current session-wide state.
/// - `detail` contains stable, sanitized reason codes (e.g. `provision_failed`,
///   `connect_failed`, `authorization_changed`, `revoke_failed`,
///   `runtime_queue_full`, `worker_unavailable`, `session_closed`), never raw
///   credentials, JWTs, database URLs, or SQL error strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticSyncStatus {
    pub phase: SemanticSyncPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl SemanticSyncStatus {
    /// The default state for sessions that are not authorized for semantic
    /// self-render synchronization.
    pub fn disabled() -> Self {
        Self {
            phase: SemanticSyncPhase::Disabled,
            source_snapshot_id: None,
            projection_hash: None,
            detail: None,
        }
    }

    /// Creates a status update without carrying forward stale projection
    /// identity after a lifecycle transition.
    pub fn phase(phase: SemanticSyncPhase, detail: Option<String>) -> Self {
        Self {
            phase,
            source_snapshot_id: None,
            projection_hash: None,
            detail,
        }
    }

    /// Creates a ready status for one verified authorized projection.
    pub fn ready(source_snapshot_id: String, projection_hash: String) -> Self {
        Self {
            phase: SemanticSyncPhase::Ready,
            source_snapshot_id: Some(source_snapshot_id),
            projection_hash: Some(projection_hash),
            detail: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_sync_status_constructors_and_serialization_match_protocol() {
        let disabled = SemanticSyncStatus::disabled();
        assert_eq!(disabled.phase, SemanticSyncPhase::Disabled);
        assert!(disabled.source_snapshot_id.is_none());
        assert!(disabled.projection_hash.is_none());
        assert!(disabled.detail.is_none());

        let json = serde_json::to_string(&disabled).unwrap();
        assert_eq!(json, r#"{"phase":"disabled"}"#);

        let failed =
            SemanticSyncStatus::phase(SemanticSyncPhase::Failed, Some("revoke_failed".to_owned()));
        let json_failed = serde_json::to_string(&failed).unwrap();
        assert_eq!(
            json_failed,
            r#"{"phase":"failed","detail":"revoke_failed"}"#
        );

        let ready = SemanticSyncStatus::ready("snap-1".to_owned(), "hash-abc".to_owned());
        let json_ready = serde_json::to_string(&ready).unwrap();
        assert_eq!(
            json_ready,
            r#"{"phase":"ready","source_snapshot_id":"snap-1","projection_hash":"hash-abc"}"#
        );

        let decoded: SemanticSyncStatus = serde_json::from_str(&json_ready).unwrap();
        assert_eq!(decoded, ready);
    }

    #[test]
    fn semantic_sync_phases_serialize_to_snake_case() {
        for (phase, expected) in [
            (SemanticSyncPhase::Disabled, "disabled"),
            (SemanticSyncPhase::Provisioning, "provisioning"),
            (SemanticSyncPhase::Provisioned, "provisioned"),
            (SemanticSyncPhase::Connecting, "connecting"),
            (SemanticSyncPhase::Ready, "ready"),
            (SemanticSyncPhase::Pulling, "pulling"),
            (SemanticSyncPhase::Pushing, "pushing"),
            (SemanticSyncPhase::Stale, "stale"),
            (SemanticSyncPhase::Failed, "failed"),
            (SemanticSyncPhase::Closed, "closed"),
        ] {
            let json = serde_json::to_string(&phase).unwrap();
            assert_eq!(json, format!("\"{expected}\""));
        }
    }
}
