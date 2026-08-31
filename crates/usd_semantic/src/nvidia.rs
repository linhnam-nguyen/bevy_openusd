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

/// Projects an observed NVIDIA/Revit raw property to source-neutral UI
/// metadata. The raw key remains unchanged and is never replaced by the
/// human-facing label.
pub fn nvidia_revit_property_descriptor(name: &str) -> BimPropertyDescriptor {
    let (scope, suffix) = if let Some(suffix) = name.strip_prefix("BIM:Instance:") {
        (BimPropertyScope::Instance, suffix)
    } else if let Some(suffix) = name.strip_prefix("BIM:Type:") {
        (BimPropertyScope::Type, suffix)
    } else {
        (BimPropertyScope::Other, name)
    };
    BimPropertyDescriptor::new(name, humanize_label(suffix), scope)
}

fn humanize_label(suffix: &str) -> String {
    match suffix {
        "ElementId" => "Element ID".to_owned(),
        "IfcGUID" => "IFC GUID".to_owned(),
        _ => suffix.to_owned(),
    }
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
    match value {
        Some(value) => {
            bytes.push(1);
            write_string(bytes, value);
        }
        None => bytes.push(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property(name: &str, value: CanonicalValue) -> SemanticProperty {
        SemanticProperty {
            name: name.to_owned(),
            value,
            measurement: None,
        }
    }

    #[test]
    fn mapping_normalizes_numeric_value_and_attaches_metadata() {
        let mut properties = vec![
            property("height", CanonicalValue::Real(10.0)),
            property("height_unit", CanonicalValue::Text("[ft_i]".to_owned())),
        ];
        let config = NvidiaRevitConfig {
            classification: NvidiaRevitClassificationConfig::default(),
            measurement_mappings: vec![NvidiaRevitMeasurementMapping::new(
                "height",
                "length",
                "height_unit",
            )],
            ..Default::default()
        };

        attach_measurements(&mut properties, &config);

        assert_eq!(properties[0].value, CanonicalValue::Real(3.048));
        let metadata = properties[0]
            .measurement
            .as_ref()
            .expect("configured measurement");
        assert_eq!(metadata.quantity.as_str(), "length");
        assert_eq!(metadata.canonical_unit.as_str(), "m");
        assert_eq!(metadata.source_unit.as_ref().unwrap().as_str(), "[ft_i]");
    }

    #[test]
    fn missing_or_unknown_units_preserve_the_typed_value_without_guessing() {
        let mut missing = vec![property("height", CanonicalValue::Real(10.0))];
        let mut unknown = vec![
            property("height", CanonicalValue::Real(10.0)),
            property(
                "height_unit",
                CanonicalValue::Text("revit:unknown".to_owned()),
            ),
        ];
        let config = NvidiaRevitConfig {
            classification: NvidiaRevitClassificationConfig::default(),
            measurement_mappings: vec![NvidiaRevitMeasurementMapping::new(
                "height",
                "length",
                "height_unit",
            )],
            ..Default::default()
        };

        attach_measurements(&mut missing, &config);
        attach_measurements(&mut unknown, &config);

        assert_eq!(missing[0].value, CanonicalValue::Real(10.0));
        assert_eq!(unknown[0].value, CanonicalValue::Real(10.0));
        assert!(missing[0].measurement.is_none());
        assert!(unknown[0].measurement.is_none());
    }

    #[test]
    fn identity_mapping_does_not_promote_category_to_family_name() {
        let properties = vec![
            property(
                "BIM:Instance:Category",
                CanonicalValue::Text("Murs".to_owned()),
            ),
            property(
                "BIM:Instance:ElementId",
                CanonicalValue::Text("150663".to_owned()),
            ),
            property(
                "BIM:Type:Name",
                CanonicalValue::Text("Générique - 200 mm".to_owned()),
            ),
        ];
        let identity = extract_bim_identity(
            &properties,
            &NvidiaRevitIdentityConfig {
                element_id_property: Some("BIM:Instance:ElementId".to_owned()),
                family_name_property: None,
            },
        );

        assert_eq!(identity.element_id.as_deref(), Some("150663"));
        assert_eq!(identity.family_name, None);
    }

    #[test]
    fn classification_mapping_keeps_generic_usd_fields_separate() {
        let properties = vec![
            property(
                "BIM:Instance:Category",
                CanonicalValue::Text("Murs".to_owned()),
            ),
            property(
                "BIM:Type:Name",
                CanonicalValue::Text("Générique - 400 mm".to_owned()),
            ),
        ];
        let classification = extract_bim_classification(
            &properties,
            &NvidiaRevitClassificationConfig {
                category_property: Some("BIM:Instance:Category".to_owned()),
                type_name_property: Some("BIM:Type:Name".to_owned()),
                ..Default::default()
            },
        );

        assert_eq!(classification.category.as_deref(), Some("Murs"));
        assert_eq!(
            classification.type_name.as_deref(),
            Some("Générique - 400 mm")
        );
        assert_eq!(
            classification.category_property.as_deref(),
            Some("BIM:Instance:Category")
        );
        assert_eq!(
            nvidia_revit_property_descriptor("BIM:Type:Description").label,
            "Description"
        );
        assert_eq!(
            nvidia_revit_property_descriptor("BIM:Type:Description").scope,
            BimPropertyScope::Type
        );
    }

    #[test]
    fn identity_mapping_accepts_only_explicit_family_name_source() {
        let properties = vec![property(
            "BIM:Instance:FamilyName",
            CanonicalValue::Text("Basic Wall".to_owned()),
        )];
        let identity = extract_bim_identity(
            &properties,
            &NvidiaRevitIdentityConfig {
                family_name_property: Some("BIM:Instance:FamilyName".to_owned()),
                ..Default::default()
            },
        );

        assert_eq!(identity.family_name.as_deref(), Some("Basic Wall"));
    }
}
