//! Transport-neutral semantic projection synchronization status.

use serde::{Deserialize, Serialize};

/// Client-requested semantic synchronization operation.
///
/// The operation intentionally carries no authorization policy, remote URL,
/// database name, or credential. The server derives authorization from the
/// established session and keeps all deployment details private.
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
