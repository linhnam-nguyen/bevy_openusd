use openusd::sdf::Value;
use openusd::usd::Stage;
use usd_model::{CanonicalValue, MeasurementMetadata, UnitId};
use viewport_protocol::EditorValue;

use crate::viewport::api::editor_value_to_usd;

use super::locator::{BimAuthoringError, BimAuthoringLocator, BimEditability};

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
        .map_err(|error| BimAuthoringError::Stage(error.to_string()))?
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
