//! Typed BIM read models returned by the application service.

use serde::{Deserialize, Serialize};
use usd_model::{CanonicalValue, MeasurementMetadata, UnitId};

use super::super::editor::EditorValue;
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
    /// Preview-only scale supplied by the authoritative registry. Clients
    /// must still send the stable unit ID; the backend resolves conversion
    /// factors independently before authoring.
    #[serde(default = "default_unit_scale")]
    pub scale_to_canonical: f64,
    /// Preview-only affine offset supplied by the authoritative registry.
    #[serde(default)]
    pub offset_to_canonical: f64,
}

fn default_unit_scale() -> f64 {
    1.0
}

/// Identifies the authoritative source group for one projected BIM property.
///
/// `SourceFallback` is reserved for validated source properties that are not
/// part of the normalized semantic property set. It is intentionally distinct
/// from `Semantic` so a client cannot silently merge provenance groups.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
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
    /// Canonical compare-and-set values aligned with the read-model targets.
    /// This is authoritative batch-edit input, not presentation text.
    #[serde(default)]
    pub target_values: Vec<CanonicalValue>,
    pub measurement: Option<MeasurementMetadata>,
    pub units: Vec<BimUnitOption>,
    /// The authoritative unit used for display and input. The UI must not
    /// offer a unit selector; this value is chosen by the backend registry.
    #[serde(default)]
    pub current_display_unit: Option<UnitId>,
    pub editable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BimPropertyGroupReadModel {
    pub id: BimPropertyGroupId,
    pub name: String,
    pub editable_group: bool,
    pub properties: Vec<BimPropertyReadModel>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BimPropertyProvenanceStatus {
    Available,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BimPropertyProvenanceReadModel {
    pub target: SceneAnchor,
    pub property: String,
    /// Git history head used to compute this result and validate cache use.
    pub history_head: String,
    pub status: BimPropertyProvenanceStatus,
    pub commit_id: Option<String>,
    pub commit_message: Option<String>,
    pub author_name: Option<String>,
    pub author_email: Option<String>,
    pub authored_at_seconds: Option<i64>,
    pub old_value: Option<CanonicalValue>,
    pub new_value: Option<CanonicalValue>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BimPropertiesReadModel {
    pub targets: Vec<SceneAnchor>,
    #[serde(default)]
    pub selection_revision: u64,
    pub groups: Vec<BimPropertyGroupReadModel>,
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

impl BimReplacementPreviewRow {
    /// Converts a typed preview value to the JSON-native editor value while
    /// preserving the canonical value's scalar/array representation.
    pub fn proposed_editor_value(&self) -> Option<EditorValue> {
        match self.proposed_canonical_value.as_ref()? {
            CanonicalValue::Null => Some(EditorValue::Null),
            CanonicalValue::Bool(value) => Some(EditorValue::Bool(*value)),
            CanonicalValue::Integer(value) => Some(EditorValue::from(*value)),
            CanonicalValue::Real(value) => {
                serde_json::Number::from_f64(*value).map(EditorValue::Number)
            }
            CanonicalValue::Text(value) => Some(EditorValue::String(value.clone())),
            CanonicalValue::TextArray(values) => serde_json::to_value(values).ok(),
            CanonicalValue::NumberArray(values) => serde_json::to_value(values).ok(),
            CanonicalValue::Json(value) => serde_json::from_str(value).ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(value: CanonicalValue) -> BimReplacementPreviewRow {
        BimReplacementPreviewRow {
            anchor: SceneAnchor::active_session("/World/Door"),
            label: "Door".to_owned(),
            property: "Mark".to_owned(),
            old_value: "A".to_owned(),
            proposed_value: "B".to_owned(),
            expected_old_value: CanonicalValue::Text("A".to_owned()),
            proposed_canonical_value: Some(value),
            measurement: None,
        }
    }

    #[test]
    fn proposed_editor_value_preserves_json_scalar_shape() {
        assert_eq!(
            row(CanonicalValue::Integer(4)).proposed_editor_value(),
            Some(EditorValue::from(4))
        );
        assert_eq!(
            row(CanonicalValue::Text("B".to_owned())).proposed_editor_value(),
            Some(EditorValue::String("B".to_owned()))
        );
        assert_eq!(
            row(CanonicalValue::Json("{\"ok\":true}".to_owned())).proposed_editor_value(),
            Some(serde_json::json!({"ok": true}))
        );
    }
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
