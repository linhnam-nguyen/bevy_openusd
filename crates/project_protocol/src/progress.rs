use serde::{Deserialize, Serialize};

/// Coarse status for one replaceable Project import operation.
///
/// Progress describes operation state, not individual files or USD prims.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectImportPhase {
    Queued,
    Inspecting,
    Preparing,
    Validating,
    Publishing,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectImportProgress {
    pub operation_id: String,
    pub generation: u64,
    pub phase: ProjectImportPhase,
}
