//! Typed BIM read models returned by the application service.

use serde::{Deserialize, Serialize};
use usd_model::{CanonicalValue, MeasurementMetadata, UnitId};

use super::super::read_models::SceneAnchor;
use super::constants::UNCLASSIFIED_LABEL;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CommonValue {
    Same(CanonicalValue),
    Multiple,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BimUnitOption {
    pub unit: UnitId,
    pub label: String,
}

/// Identifies the authoritative source group for one projected BIM property.
///
/// `SourceFallback` is reserved for validated source properties that are not
/// part of the normalized semantic property set. It is intentionally distinct
/// from `Semantic` so a client cannot silently merge provenance groups.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BimPropertyGroupId {
    #[default]
    Semantic,
    SourceFallback,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BimPropertyReadModel {
    pub key: String,
    #[serde(default)]
    pub group_id: BimPropertyGroupId,
    pub value: CommonValue,
    pub measurement: Option<MeasurementMetadata>,
    pub units: Vec<BimUnitOption>,
    pub editable: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BimPropertiesReadModel {
    pub targets: Vec<SceneAnchor>,
    #[serde(default)]
    pub selection_revision: u64,
    pub properties: Vec<BimPropertyReadModel>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BimPropertyNameMatch {
    pub name: String,
    pub measurement: Option<MeasurementMetadata>,
    pub object_count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BimPropertyValueMatch {
    pub name: String,
    pub value: CanonicalValue,
    pub display_value: String,
    pub measurement: Option<MeasurementMetadata>,
    pub object_count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BimObjectMatch {
    pub anchor: SceneAnchor,
    pub label: String,
    pub property: String,
    pub value: CanonicalValue,
    pub display_value: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BimReplacementPreviewRow {
    pub anchor: SceneAnchor,
    pub label: String,
    pub property: String,
    pub old_value: String,
    pub proposed_value: String,
    /// The typed compare-and-set value captured when the preview was built.
    pub expected_old_value: CanonicalValue,
    /// `None` means the replacement text cannot be represented by the
    /// original semantic value type and must be rejected on Apply.
    pub proposed_canonical_value: Option<CanonicalValue>,
    pub measurement: Option<MeasurementMetadata>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum BimSearchResult {
    PropertyNames {
        offset: u32,
        total: u32,
        matches: Vec<BimPropertyNameMatch>,
        has_more: bool,
    },
    PropertyValues {
        offset: u32,
        total: u32,
        matches: Vec<BimPropertyValueMatch>,
        has_more: bool,
    },
    Objects {
        offset: u32,
        total: u32,
        matches: Vec<BimObjectMatch>,
        has_more: bool,
    },
    ReplacementPreview {
        offset: u32,
        total: u32,
        rows: Vec<BimReplacementPreviewRow>,
        has_more: bool,
    },
}

impl BimSearchResult {
    pub const fn unclassified_label() -> &'static str {
        UNCLASSIFIED_LABEL
    }
}
