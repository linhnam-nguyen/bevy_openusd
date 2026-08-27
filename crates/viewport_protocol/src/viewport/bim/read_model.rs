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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BimPropertyReadModel {
    pub key: String,
    pub value: CommonValue,
    pub measurement: Option<MeasurementMetadata>,
    pub units: Vec<BimUnitOption>,
    pub editable: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BimPropertiesReadModel {
    pub targets: Vec<SceneAnchor>,
    pub properties: Vec<BimPropertyReadModel>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClassificationNodeReadModel {
    pub id: String,
    pub parent_id: Option<String>,
    pub level: u32,
    pub label: String,
    pub entity_count: u32,
    pub has_children: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClassificationLeafReadModel {
    pub anchor: SceneAnchor,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ClassificationRow {
    Group(ClassificationNodeReadModel),
    Leaf(ClassificationLeafReadModel),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClassificationChildrenPage {
    pub parent_id: Option<String>,
    pub page: u32,
    pub page_size: u32,
    pub total: u32,
    pub rows: Vec<ClassificationRow>,
    pub has_more: bool,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BimReplacementPreviewRow {
    pub anchor: SceneAnchor,
    pub label: String,
    pub property: String,
    pub old_value: String,
    pub proposed_value: String,
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
