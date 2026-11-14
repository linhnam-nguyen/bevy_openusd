//! Common semantic fields used for search and grouping.

use serde::{Deserialize, Serialize};

/// Source-neutral scope for a BIM property after exporter-specific
/// normalization. `Other` deliberately means that no validated instance/type
/// distinction was observed.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum BimPropertyScope {
    Instance,
    Type,
    #[default]
    Other,
}

/// Human-facing projection metadata for one raw BIM property. The raw key is
/// retained as the edit/search/provenance authority; label and scope are only
/// presentation metadata.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct BimPropertyDescriptor {
    pub key: String,
    pub label: String,
    pub scope: BimPropertyScope,
}

impl BimPropertyDescriptor {
    pub fn new(key: impl Into<String>, label: impl Into<String>, scope: BimPropertyScope) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            scope,
        }
    }
}

/// Source-neutral BIM identity values normalized by an observed exporter
/// adapter. An absent value means the source did not provide validated
/// evidence for that identity; it must not be inferred from another field.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct BimIdentity {
    pub element_id: Option<String>,
    pub family_name: Option<String>,
}

/// Normalized Revit/BIM classification values. These are intentionally kept
/// separate from generic OpenUSD `kind` and prim schema `type_name` fields.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct BimClassificationInfo {
    pub category: Option<String>,
    pub family_name: Option<String>,
    pub type_name: Option<String>,
    /// Raw source keys used to produce the normalized values. They allow the
    /// read-model layer to suppress duplicate convenience aliases without
    /// making the UI understand exporter namespaces.
    pub category_property: Option<String>,
    pub family_name_property: Option<String>,
    pub type_name_property: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SemanticInfo {
    pub category: Option<String>,
    pub family: Option<String>,
    pub type_name: Option<String>,
    pub type_id: Option<String>,
    pub display_name: Option<String>,
    /// Normalized BIM identities, kept separate from generic semantic fields.
    #[serde(default)]
    pub bim: BimIdentity,
    /// Normalized BIM classification, kept separate from generic USD fields.
    #[serde(default)]
    pub bim_classification: BimClassificationInfo,
}

impl SemanticInfo {
    /// Returns the authoritative BIM eligibility fact produced by a semantic
    /// exporter adapter. Generic USD metadata is deliberately not enough to
    /// opt an entity into BIM classification or search.
    pub fn is_bim_entity(&self) -> bool {
        is_non_empty(self.bim.element_id.as_deref())
            || is_non_empty(self.bim.family_name.as_deref())
    }
}

fn is_non_empty(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

#[cfg(test)]
#[path = "../tests/unit/semantic_tests.rs"]
mod tests;
