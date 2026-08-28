//! Common semantic fields used for search and grouping.

use serde::{Deserialize, Serialize};

/// Source-neutral BIM identity values normalized by an observed exporter
/// adapter. An absent value means the source did not provide validated
/// evidence for that identity; it must not be inferred from another field.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct BimIdentity {
    pub element_id: Option<String>,
    pub family_name: Option<String>,
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
}
