use serde::{Deserialize, Serialize};
use usd_model::{CanonicalValue, MeasurementMetadata};

use super::super::read_models::SceneAnchor;

/// Property-level state relative to the selected entity's Git baseline.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BimPropertyDiffStatus {
    Unchanged,
    Modified,
    Added,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BimPropertyDiffRow {
    pub key: String,
    pub status: BimPropertyDiffStatus,
    pub old_value: Option<CanonicalValue>,
    pub new_value: Option<CanonicalValue>,
    pub old_measurement: Option<MeasurementMetadata>,
    pub new_measurement: Option<MeasurementMetadata>,
}

/// Bounded single-selection property diff read model.
///
/// `base_git_oid` is explicit so a client cannot mistake a manually captured
/// working snapshot for the Git baseline used for diff styling.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BimPropertyDiffReadModel {
    pub target: SceneAnchor,
    pub base_git_oid: String,
    pub working_snapshot_id: String,
    pub status: BimPropertyDiffStatus,
    pub properties: Vec<BimPropertyDiffRow>,
}
