use usd_model::{EntitySnapshot, HashDigest, SnapshotId, SnapshotSource};

use super::query::SemanticQueryResult;

#[derive(Debug)]
pub(crate) struct SemanticIncrementalUpdate {
    pub(crate) snapshot_id: SnapshotId,
    pub(crate) source: SnapshotSource,
    pub(crate) config_hash: HashDigest,
    pub(crate) upserts: Vec<EntitySnapshot>,
    pub(crate) removed_paths: Vec<String>,
}

#[derive(Debug)]
pub(crate) enum SemanticResponse {
    SnapshotLoaded {
        request_id: String,
        entity_count: u32,
    },
    DeltaApplied {
        request_id: String,
        upserted: u32,
        removed: u32,
    },
    QueryResult {
        request_id: String,
        result: SemanticQueryResult,
    },
    Failed {
        request_id: String,
        operation: &'static str,
        error: String,
    },
}
