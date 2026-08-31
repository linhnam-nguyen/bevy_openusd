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
mod tests {
    use super::*;

    #[test]
    fn bim_eligibility_requires_normalized_identity_evidence() {
        let generic = SemanticInfo {
            category: Some("Camera".to_owned()),
            type_name: Some("Camera".to_owned()),
            display_name: Some("Helper camera".to_owned()),
            ..Default::default()
        };
        assert!(!generic.is_bim_entity());

        let mut bim = generic;
        bim.bim.family_name = Some("Window family".to_owned());
        assert!(bim.is_bim_entity());
    }

    #[test]
    fn whitespace_only_normalized_identity_is_not_eligibility_evidence() {
        let semantic = SemanticInfo {
            bim: BimIdentity {
                element_id: Some("  ".to_owned()),
                family_name: Some("\t".to_owned()),
            },
            ..Default::default()
        };
        assert!(!semantic.is_bim_entity());
    }
}
