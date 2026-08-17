//! Extraction tolerances and metadata selection.

use usd_model::HashDigest;

/// Candidate authored-property names for resolving a stable source identity.
///
/// The vectors are deliberately empty by default. Exporters disagree on
/// namespace and spelling, so a resolver must be configured from observed
/// source data instead of silently relying on guessed Revit/IFC names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdentityConfig {
    /// Revit UniqueId candidates, checked in order.
    pub revit_unique_id_candidates: Vec<String>,
    /// IFC GUID candidates, checked after Revit candidates.
    pub ifc_guid_candidates: Vec<String>,
    /// Application-owned GUID candidates, checked after IFC candidates.
    pub application_guid_candidates: Vec<String>,
    /// Explicit asset/application identifier candidates.
    pub asset_identifier_candidates: Vec<String>,
    /// Use the composed prim path when no configured source identifier exists.
    pub allow_prim_path_fallback: bool,
    /// Use a deterministic hash when path fallback is disabled.
    pub allow_synthetic_fallback: bool,
}

impl IdentityConfig {
    fn write_hash(&self, bytes: &mut Vec<u8>) {
        for candidates in [
            &self.revit_unique_id_candidates,
            &self.ifc_guid_candidates,
            &self.application_guid_candidates,
            &self.asset_identifier_candidates,
        ] {
            bytes.extend_from_slice(&(candidates.len() as u64).to_le_bytes());
            for candidate in candidates {
                bytes.extend_from_slice(&(candidate.len() as u64).to_le_bytes());
                bytes.extend_from_slice(candidate.as_bytes());
            }
        }
        bytes.push(u8::from(self.allow_prim_path_fallback));
        bytes.push(u8::from(self.allow_synthetic_fallback));
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticConfig {
    /// Number of millimetres represented by one stage unit.
    pub translation_mm_per_unit: f64,
    /// Quantization multiplier for quaternion components.
    pub rotation_quantization: f64,
    /// Quantization multiplier for scale components.
    pub scale_quantization: f64,
    /// Quantization multiplier for local geometry coordinates.
    pub geometry_quantization: f64,
    pub include_custom_properties: bool,
    pub family_property: Option<String>,
    pub type_id_property: Option<String>,
    pub display_name_property: Option<String>,
    pub identity: IdentityConfig,
}

impl Default for SemanticConfig {
    fn default() -> Self {
        Self {
            translation_mm_per_unit: 1_000.0,
            rotation_quantization: 100_000.0,
            scale_quantization: 10_000.0,
            geometry_quantization: 1_000_000.0,
            include_custom_properties: true,
            family_property: None,
            type_id_property: None,
            display_name_property: None,
            identity: IdentityConfig {
                allow_prim_path_fallback: true,
                ..Default::default()
            },
        }
    }
}

impl SemanticConfig {
    pub(crate) fn hash(&self) -> HashDigest {
        let mut bytes = Vec::new();
        for value in [
            self.translation_mm_per_unit,
            self.rotation_quantization,
            self.scale_quantization,
            self.geometry_quantization,
        ] {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        bytes.push(u8::from(self.include_custom_properties));
        for value in [
            self.family_property.as_deref(),
            self.type_id_property.as_deref(),
            self.display_name_property.as_deref(),
        ] {
            match value {
                Some(value) => {
                    bytes.push(1);
                    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
                    bytes.extend_from_slice(value.as_bytes());
                }
                None => bytes.push(0),
            }
        }
        self.identity.write_hash(&mut bytes);
        HashDigest::new(*blake3::hash(&bytes).as_bytes())
    }
}
