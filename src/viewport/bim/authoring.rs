//! Stage-backed BIM property authoring locators.
//!
//! A semantic property name is a stable key supplied by the extractor. It is
//! not a display label and it is not converted through the Bevy scene index.
//! This module resolves that key directly against the current OpenUSD stage
//! before any edit is admitted.

use std::fmt;

use openusd::sdf::Value;
use openusd::usd::Stage;
use usd_model::{CanonicalValue, MeasurementMetadata, UnitId};
use viewport_protocol::{EditorValue, SceneAnchor};

use crate::viewport::api::editor_value_to_usd;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BimEditability {
    Editable,
    NonEditable { reason: BimNonEditableReason },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BimNonEditableReason {
    DerivedProperty,
    NonCustomAttribute,
    UnsupportedType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BimAuthoringLocator {
    pub(crate) target: SceneAnchor,
    pub(crate) property_key: String,
    pub(crate) prim_path: String,
    pub(crate) attribute_path: String,
    pub(crate) type_name: Option<String>,
    pub(crate) editability: BimEditability,
}

impl BimAuthoringLocator {
    pub(crate) fn is_editable(&self) -> bool {
        self.editability == BimEditability::Editable
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum BimAuthoringError {
    InvalidPropertyKey,
    InvalidPrimPath(String),
    PrimNotFound(String),
    AttributeNotFound {
        prim_path: String,
        property_key: String,
    },
    AttributeValueMissing {
        attribute_path: String,
    },
    ExpectedValueMismatch {
        property_key: String,
        expected: CanonicalValue,
        current: CanonicalValue,
    },
    NonEditable {
        property_key: String,
        reason: BimNonEditableReason,
    },
    InvalidUnit(String),
    InvalidValue(String),
    Stage(String),
}

impl fmt::Display for BimAuthoringError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPropertyKey => formatter.write_str("BIM property key is invalid"),
            Self::InvalidPrimPath(path) => write!(formatter, "invalid BIM prim path: {path}"),
            Self::PrimNotFound(path) => write!(formatter, "BIM prim not found: {path}"),
            Self::AttributeNotFound {
                prim_path,
                property_key,
            } => write!(
                formatter,
                "BIM property {property_key} is missing on prim {prim_path}"
            ),
            Self::AttributeValueMissing { attribute_path } => {
                write!(
                    formatter,
                    "BIM property has no authored value: {attribute_path}"
                )
            }
            Self::ExpectedValueMismatch {
                property_key,
                expected,
                current,
            } => write!(
                formatter,
                "BIM property {property_key} is stale: expected {expected:?}, current {current:?}"
            ),
            Self::NonEditable {
                property_key,
                reason,
            } => write!(
                formatter,
                "BIM property {property_key} is not editable ({reason:?})"
            ),
            Self::InvalidUnit(error) => write!(formatter, "invalid BIM edit unit: {error}"),
            Self::InvalidValue(error) => write!(formatter, "invalid BIM edit value: {error}"),
            Self::Stage(error) => write!(formatter, "BIM stage inspection failed: {error}"),
        }
    }
}

pub(crate) fn current_bim_value(
    stage: &Stage,
    locator: &BimAuthoringLocator,
) -> Result<Value, BimAuthoringError> {
    let attribute = stage
        .prim(
            openusd::sdf::path(&locator.prim_path)
                .map_err(|_| BimAuthoringError::InvalidPrimPath(locator.prim_path.clone()))?,
        )
        .attribute(&locator.property_key);
    attribute
        .get::<Value>()
        .map_err(stage_error)?
        .ok_or_else(|| BimAuthoringError::AttributeValueMissing {
            attribute_path: locator.attribute_path.clone(),
        })
}

pub(crate) fn canonical_value_for_comparison(
    value: Value,
    measurement: Option<&MeasurementMetadata>,
) -> Result<CanonicalValue, BimAuthoringError> {
    let value = usd_semantic::canonical_value(value);
    let Some(measurement) = measurement else {
        return Ok(value);
    };
    let Some(source_unit) = measurement.source_unit.as_ref() else {
        return Ok(value);
    };
    map_numeric_value(&value, |number| {
        usd_semantic::UnitRegistry::global()
            .convert(number, source_unit, &measurement.canonical_unit)
            .map_err(|error| error.to_string())
    })
}

pub(crate) fn prepare_bim_value(
    locator: &BimAuthoringLocator,
    input: &EditorValue,
    input_unit: Option<&UnitId>,
    measurement: Option<&MeasurementMetadata>,
) -> Result<(Value, CanonicalValue), BimAuthoringError> {
    if !locator.is_editable() {
        let reason = match locator.editability {
            BimEditability::Editable => unreachable!("editable locator has no reason"),
            BimEditability::NonEditable { reason } => reason,
        };
        return Err(BimAuthoringError::NonEditable {
            property_key: locator.property_key.clone(),
            reason,
        });
    }
    let type_name = locator
        .type_name
        .as_deref()
        .ok_or_else(|| BimAuthoringError::InvalidValue("attribute has no USD type".to_owned()))?;

    let authored_input = match measurement {
        None => {
            if input_unit.is_some() {
                return Err(BimAuthoringError::InvalidUnit(
                    "a unit is only valid for measurable properties".to_owned(),
                ));
            }
            input.clone()
        }
        Some(measurement) => {
            let input_unit = input_unit.ok_or_else(|| {
                BimAuthoringError::InvalidUnit(
                    "a measurable property requires an explicit input unit".to_owned(),
                )
            })?;
            let registry = usd_semantic::UnitRegistry::global();
            let input_definition = registry.definition(input_unit).ok_or_else(|| {
                BimAuthoringError::InvalidUnit(format!("unknown unit {}", input_unit.as_str()))
            })?;
            if input_definition.quantity().as_str() != measurement.quantity.as_str() {
                return Err(BimAuthoringError::InvalidUnit(format!(
                    "unit {} does not measure {}",
                    input_unit.as_str(),
                    measurement.quantity.as_str()
                )));
            }
            let author_unit = measurement
                .source_unit
                .as_ref()
                .unwrap_or(&measurement.canonical_unit);
            rewrite_numeric_json(input, type_name, |number| {
                let canonical = registry
                    .convert(number, input_unit, &measurement.canonical_unit)
                    .map_err(|error| error.to_string())?;
                registry
                    .convert(canonical, &measurement.canonical_unit, author_unit)
                    .map_err(|error| error.to_string())
            })?
        }
    };
    let authored =
        editor_value_to_usd(type_name, &authored_input).map_err(BimAuthoringError::InvalidValue)?;
    let canonical = canonical_value_for_comparison(authored.clone(), measurement)?;
    Ok((authored, canonical))
}

fn map_numeric_value(
    value: &CanonicalValue,
    convert: impl Fn(f64) -> Result<f64, String> + Copy,
) -> Result<CanonicalValue, BimAuthoringError> {
    match value {
        CanonicalValue::Integer(value) => convert(*value as f64)
            .map(CanonicalValue::Real)
            .map_err(BimAuthoringError::InvalidValue),
        CanonicalValue::Real(value) => convert(*value)
            .map(CanonicalValue::Real)
            .map_err(BimAuthoringError::InvalidValue),
        CanonicalValue::NumberArray(values) => values
            .iter()
            .copied()
            .map(convert)
            .collect::<Result<Vec<_>, _>>()
            .map(CanonicalValue::NumberArray)
            .map_err(BimAuthoringError::InvalidValue),
        _ => Err(BimAuthoringError::InvalidValue(
            "measurable BIM properties must contain numeric values".to_owned(),
        )),
    }
}

fn rewrite_numeric_json(
    value: &EditorValue,
    type_name: &str,
    convert: impl Fn(f64) -> Result<f64, String> + Copy,
) -> Result<EditorValue, BimAuthoringError> {
    if let Some(values) = value.as_array() {
        return values
            .iter()
            .map(|value| rewrite_numeric_json(value, type_name, convert))
            .collect::<Result<Vec<_>, _>>()
            .map(EditorValue::Array);
    }
    let input = value
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| {
            BimAuthoringError::InvalidValue("measurable input must be numeric".to_owned())
        })?;
    let converted = convert(input).map_err(BimAuthoringError::InvalidValue)?;
    if !converted.is_finite() {
        return Err(BimAuthoringError::InvalidValue(
            "converted BIM value is not finite".to_owned(),
        ));
    }
    if is_integer_type(type_name) {
        if converted.fract() != 0.0 {
            return Err(BimAuthoringError::InvalidValue(format!(
                "converted value {converted} is not an integer for {type_name}"
            )));
        }
        let integer = converted as i128;
        if !((i64::MIN as i128)..=(u64::MAX as i128)).contains(&integer) {
            return Err(BimAuthoringError::InvalidValue(
                "converted integer is outside USD range".to_owned(),
            ));
        }
        if type_name.starts_with("uint") && integer < 0 {
            return Err(BimAuthoringError::InvalidValue(
                "converted unsigned value is negative".to_owned(),
            ));
        }
        return Ok(EditorValue::Number(if type_name.starts_with("uint") {
            serde_json::Number::from(integer as u64)
        } else {
            serde_json::Number::from(integer as i64)
        }));
    }
    serde_json::Number::from_f64(converted)
        .map(EditorValue::Number)
        .ok_or_else(|| BimAuthoringError::InvalidValue("converted value is not finite".to_owned()))
}

fn is_integer_type(type_name: &str) -> bool {
    type_name.starts_with("int") || type_name.starts_with("uint") || type_name == "uchar"
}

impl std::error::Error for BimAuthoringError {}

pub(crate) fn resolve_bim_authoring_locator(
    stage: &Stage,
    target: &SceneAnchor,
    property_key: &str,
) -> Result<BimAuthoringLocator, BimAuthoringError> {
    target
        .validate()
        .map_err(|_| BimAuthoringError::InvalidPrimPath(target.prim_path.clone()))?;
    if property_key.trim().is_empty() || property_key.contains('\0') {
        return Err(BimAuthoringError::InvalidPropertyKey);
    }

    let prim_path = openusd::sdf::path(&target.prim_path)
        .map_err(|_| BimAuthoringError::InvalidPrimPath(target.prim_path.clone()))?;
    let prim = stage.prim(prim_path);
    if !prim.is_valid().map_err(stage_error)? {
        return Err(BimAuthoringError::PrimNotFound(target.prim_path.clone()));
    }

    let attribute_path = format!("{}.{}", target.prim_path, property_key);
    if is_derived_property(property_key) {
        return Ok(BimAuthoringLocator {
            target: target.clone(),
            property_key: property_key.to_owned(),
            prim_path: target.prim_path.clone(),
            attribute_path,
            type_name: None,
            editability: BimEditability::NonEditable {
                reason: BimNonEditableReason::DerivedProperty,
            },
        });
    }

    let has_attribute = prim
        .property_names()
        .map_err(stage_error)?
        .iter()
        .any(|name| name.as_str() == property_key);
    if !has_attribute {
        return Err(BimAuthoringError::AttributeNotFound {
            prim_path: target.prim_path.clone(),
            property_key: property_key.to_owned(),
        });
    }

    let attribute = prim.attribute(property_key);
    let type_name = attribute
        .type_name()
        .map_err(stage_error)?
        .map(|name| name.as_str().to_owned());
    let editability = if !attribute.is_custom().map_err(stage_error)? {
        BimEditability::NonEditable {
            reason: BimNonEditableReason::NonCustomAttribute,
        }
    } else if !type_name
        .as_deref()
        .is_some_and(is_supported_attribute_type)
    {
        BimEditability::NonEditable {
            reason: BimNonEditableReason::UnsupportedType,
        }
    } else {
        BimEditability::Editable
    };

    Ok(BimAuthoringLocator {
        target: target.clone(),
        property_key: property_key.to_owned(),
        prim_path: target.prim_path.clone(),
        attribute_path,
        type_name,
        editability,
    })
}

fn is_derived_property(property_key: &str) -> bool {
    matches!(
        property_key,
        "semantic.category"
            | "semantic.family"
            | "semantic.type_name"
            | "semantic.type_id"
            | "semantic.display_name"
            | "bim.element_id"
            | "bim.family_name"
    )
}

/// The supported set is intentionally the same scalar/vector/array surface
/// accepted by the viewport editor value converter. A resolver must not mark
/// a property editable when the normal authoring path cannot encode it.
fn is_supported_attribute_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "bool"
            | "uchar"
            | "int"
            | "uint"
            | "int64"
            | "uint64"
            | "float"
            | "double"
            | "string"
            | "token"
            | "asset"
            | "timecode"
            | "float2"
            | "float3"
            | "point3f"
            | "vector3f"
            | "normal3f"
            | "color3f"
            | "float4"
            | "color4f"
            | "double2"
            | "double3"
            | "point3d"
            | "vector3d"
            | "normal3d"
            | "color3d"
            | "double4"
            | "color4d"
            | "int2"
            | "int3"
            | "int4"
            | "quatf"
            | "quatd"
            | "matrix2d"
            | "matrix3d"
            | "matrix4d"
            | "path"
            | "bool[]"
            | "int[]"
            | "uint[]"
            | "int64[]"
            | "uint64[]"
            | "float[]"
            | "double[]"
            | "string[]"
            | "token[]"
            | "asset[]"
            | "float3[]"
            | "double3[]"
            | "matrix4d[]"
    )
}

fn stage_error(error: impl fmt::Display) -> BimAuthoringError {
    BimAuthoringError::Stage(error.to_string())
}

#[cfg(test)]
mod tests {
    use openusd::sdf::Value;
    use viewport_protocol::SceneAnchor;

    use super::*;

    fn stage_with_attribute(type_name: &str, custom: bool) -> Stage {
        let stage = Stage::builder()
            .in_memory("bim_authoring_locator.usda")
            .expect("stage opens");
        stage
            .define_prim("/World/Wall")
            .expect("prim defines")
            .set_type_name("Xform")
            .expect("prim type authors");
        stage
            .prim(openusd::sdf::path("/World/Wall").expect("path parses"))
            .create_attribute("Width", type_name)
            .expect("attribute creates")
            .set_custom(custom)
            .expect("custom flag authors")
            .set(Value::Double(1.0))
            .expect("value authors");
        stage
    }

    fn target() -> SceneAnchor {
        SceneAnchor::active_session("/World/Wall")
    }

    #[test]
    fn resolves_editable_custom_attribute_by_stable_key() {
        let stage = stage_with_attribute("double", true);
        let locator = resolve_bim_authoring_locator(&stage, &target(), "Width")
            .expect("custom scalar resolves");

        assert_eq!(locator.property_key, "Width");
        assert_eq!(locator.attribute_path, "/World/Wall.Width");
        assert_eq!(locator.type_name.as_deref(), Some("double"));
        assert!(locator.is_editable());
    }

    #[test]
    fn derived_semantic_property_is_explicitly_non_editable() {
        let stage = stage_with_attribute("double", true);
        let locator = resolve_bim_authoring_locator(&stage, &target(), "semantic.category")
            .expect("derived field resolves as a capability descriptor");

        assert_eq!(
            locator.editability,
            BimEditability::NonEditable {
                reason: BimNonEditableReason::DerivedProperty
            }
        );
        assert!(!locator.is_editable());
    }

    #[test]
    fn missing_attribute_is_rejected_without_guessing_a_target() {
        let stage = stage_with_attribute("double", true);
        assert!(matches!(
            resolve_bim_authoring_locator(&stage, &target(), "Missing"),
            Err(BimAuthoringError::AttributeNotFound { .. })
        ));
    }

    #[test]
    fn non_custom_and_unsupported_attributes_are_not_editable() {
        let non_custom = stage_with_attribute("double", false);
        let locator = resolve_bim_authoring_locator(&non_custom, &target(), "Width")
            .expect("schema attribute resolves");
        assert_eq!(
            locator.editability,
            BimEditability::NonEditable {
                reason: BimNonEditableReason::NonCustomAttribute
            }
        );

        let unsupported = stage_with_attribute("dictionary", true);
        let locator = resolve_bim_authoring_locator(&unsupported, &target(), "Width")
            .expect("unsupported attribute resolves");
        assert_eq!(
            locator.editability,
            BimEditability::NonEditable {
                reason: BimNonEditableReason::UnsupportedType
            }
        );
    }

    #[test]
    fn measured_edit_converts_input_to_the_authored_source_unit() {
        let stage = stage_with_attribute("double", true);
        let locator = resolve_bim_authoring_locator(&stage, &target(), "Width")
            .expect("measured attribute resolves");
        let measurement = MeasurementMetadata::new("length", "m", Some("mm".to_owned()));

        let (authored, canonical) = prepare_bim_value(
            &locator,
            &serde_json::json!(0.2),
            Some(&UnitId::new("m")),
            Some(&measurement),
        )
        .expect("metres convert to source millimetres");
        assert!(matches!(authored, Value::Double(value) if (value - 200.0).abs() < 1e-9));
        assert_eq!(canonical, CanonicalValue::Real(0.2));

        let current = canonical_value_for_comparison(Value::Double(200.0), Some(&measurement))
            .expect("source value normalizes to canonical metres");
        assert_eq!(current, CanonicalValue::Real(0.2));
    }

    #[test]
    fn measured_edit_rejects_unknown_or_wrong_quantity_units() {
        let stage = stage_with_attribute("double", true);
        let locator = resolve_bim_authoring_locator(&stage, &target(), "Width")
            .expect("measured attribute resolves");
        let measurement = MeasurementMetadata::new("length", "m", Some("mm".to_owned()));

        assert!(matches!(
            prepare_bim_value(
                &locator,
                &serde_json::json!(1.0),
                Some(&UnitId::new("unknown")),
                Some(&measurement),
            ),
            Err(BimAuthoringError::InvalidUnit(_))
        ));
        assert!(matches!(
            prepare_bim_value(
                &locator,
                &serde_json::json!(1.0),
                Some(&UnitId::new("Pa")),
                Some(&measurement),
            ),
            Err(BimAuthoringError::InvalidUnit(_))
        ));
    }
}
