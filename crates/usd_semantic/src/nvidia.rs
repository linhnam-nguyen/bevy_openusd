//! Explicit mapping for observed NVIDIA Revit Connector USD properties.

use usd_model::{
    BimClassificationInfo, BimIdentity, BimPropertyDescriptor, BimPropertyScope, CanonicalValue,
    MeasurementMetadata, QuantitySpecId, SemanticProperty, UnitId,
};

use crate::units::UnitRegistry;

/// A source-observed relationship between a BIM value property and its unit
/// property. There is intentionally no default mapping: connector schemas are
/// version- and export-setting-sensitive and must be configured from evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NvidiaRevitMeasurementMapping {
    pub property_name: String,
    pub quantity: QuantitySpecId,
    pub source_unit_property_name: String,
}

impl NvidiaRevitMeasurementMapping {
    pub fn new(
        property_name: impl Into<String>,
        quantity: impl Into<String>,
        source_unit_property_name: impl Into<String>,
    ) -> Self {
        Self {
            property_name: property_name.into(),
            quantity: QuantitySpecId::new(quantity),
            source_unit_property_name: source_unit_property_name.into(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NvidiaRevitConfig {
    pub identity: NvidiaRevitIdentityConfig,
    pub classification: NvidiaRevitClassificationConfig,
    pub measurement_mappings: Vec<NvidiaRevitMeasurementMapping>,
}

/// Explicit source-property mapping for the source-neutral BIM identity.
/// There are no defaults because connector schemas vary by export settings.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NvidiaRevitIdentityConfig {
    pub element_id_property: Option<String>,
    pub family_name_property: Option<String>,
}

/// Explicit source-property mapping for normalized Revit classification.
/// Empty fields remain unavailable; no value is inferred from generic USD
/// kind, prim type, category, or family fields.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NvidiaRevitClassificationConfig {
    pub category_property: Option<String>,
    pub family_name_property: Option<String>,
    pub type_name_property: Option<String>,
}

impl NvidiaRevitConfig {
    pub(crate) fn write_hash(&self, bytes: &mut Vec<u8>) {
        write_optional_string(bytes, self.identity.element_id_property.as_deref());
        write_optional_string(bytes, self.identity.family_name_property.as_deref());
        write_optional_string(bytes, self.classification.category_property.as_deref());
        write_optional_string(bytes, self.classification.family_name_property.as_deref());
        write_optional_string(bytes, self.classification.type_name_property.as_deref());
        bytes.extend_from_slice(&(self.measurement_mappings.len() as u64).to_le_bytes());
        for mapping in &self.measurement_mappings {
            write_string(bytes, &mapping.property_name);
            write_string(bytes, mapping.quantity.as_str());
            write_string(bytes, &mapping.source_unit_property_name);
        }
    }
}

pub(crate) fn extract_bim_identity(
    properties: &[SemanticProperty],
    config: &NvidiaRevitIdentityConfig,
) -> BimIdentity {
    BimIdentity {
        element_id: configured_text(properties, config.element_id_property.as_deref()),
        family_name: configured_text(properties, config.family_name_property.as_deref()),
    }
}

pub(crate) fn extract_bim_classification(
    properties: &[SemanticProperty],
    config: &NvidiaRevitClassificationConfig,
) -> BimClassificationInfo {
    BimClassificationInfo {
        category: configured_text(properties, config.category_property.as_deref()),
        family_name: configured_text(properties, config.family_name_property.as_deref()),
        type_name: configured_text(properties, config.type_name_property.as_deref()),
        category_property: config.category_property.clone(),
        family_name_property: config.family_name_property.clone(),
        type_name_property: config.type_name_property.clone(),
    }
}

/// Projects an observed NVIDIA/Revit raw property to source-neutral UI metadata
/// with an optional authored display name.
pub fn nvidia_revit_property_descriptor_with_display_name(
    name: &str,
    display_name: Option<&str>,
) -> BimPropertyDescriptor {
    let (scope, suffix) = if let Some(suffix) = name.strip_prefix("BIM:Instance:") {
        (BimPropertyScope::Instance, suffix)
    } else if let Some(suffix) = name.strip_prefix("BIM:Type:") {
        (BimPropertyScope::Type, suffix)
    } else {
        (BimPropertyScope::Other, name)
    };

    let label = match display_name.map(str::trim).filter(|text| !text.is_empty()) {
        Some(authored) => authored.to_owned(),
        None => suffix.to_owned(),
    };

    BimPropertyDescriptor::new(name, label, scope)
}

/// Projects an observed NVIDIA/Revit raw property to source-neutral UI metadata.
pub fn nvidia_revit_property_descriptor(name: &str) -> BimPropertyDescriptor {
    nvidia_revit_property_descriptor_with_display_name(name, None)
}

pub(crate) fn attach_measurements(properties: &mut [SemanticProperty], config: &NvidiaRevitConfig) {
    let registry = UnitRegistry::global();
    for mapping in &config.measurement_mappings {
        let Some(property_index) = properties
            .iter()
            .position(|property| property.name == mapping.property_name)
        else {
            continue;
        };
        let Some(source_unit) = properties
            .iter()
            .find(|property| property.name == mapping.source_unit_property_name)
            .and_then(text_value)
            .map(UnitId::new)
        else {
            continue;
        };
        let Some(metadata) = registry.metadata_for(&mapping.quantity, &source_unit).ok() else {
            continue;
        };
        let Some(value) = normalize_value(
            &properties[property_index].value,
            registry,
            &source_unit,
            &metadata,
        ) else {
            continue;
        };
        properties[property_index].value = value;
        properties[property_index].measurement = Some(metadata);
    }
}

fn text_value(property: &SemanticProperty) -> Option<&str> {
    match &property.value {
        CanonicalValue::Text(value) => Some(value),
        _ => None,
    }
}

fn configured_text(properties: &[SemanticProperty], property: Option<&str>) -> Option<String> {
    let property = property?;
    properties
        .iter()
        .find(|candidate| candidate.name == property)
        .and_then(text_value)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn normalize_value(
    value: &CanonicalValue,
    registry: &UnitRegistry,
    source_unit: &UnitId,
    metadata: &MeasurementMetadata,
) -> Option<CanonicalValue> {
    match value {
        CanonicalValue::Integer(value) => registry
            .convert(*value as f64, source_unit, &metadata.canonical_unit)
            .ok()
            .map(CanonicalValue::Real),
        CanonicalValue::Real(value) => registry
            .convert(*value, source_unit, &metadata.canonical_unit)
            .ok()
            .map(CanonicalValue::Real),
        CanonicalValue::NumberArray(values) => values
            .iter()
            .map(|value| registry.convert(*value, source_unit, &metadata.canonical_unit))
            .collect::<Result<Vec<_>, _>>()
            .ok()
            .map(CanonicalValue::NumberArray),
        _ => None,
    }
}

fn write_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

fn write_optional_string(bytes: &mut Vec<u8>, value: Option<&str>) {
    if let Some(value) = value {
        bytes.push(1);
        write_string(bytes, value);
    } else {
        bytes.push(0);
    }
}

#[cfg(test)]
#[path = "../tests/unit/nvidia_tests.rs"]
mod tests;
