//! Semantic fields and canonical custom-property extraction.

use anyhow::Result;
use openusd::schemas::ui::SceneGraphPrimAPI;
use openusd::sdf::{Path, Value};
use openusd::tf::Token;
use openusd::usd::Stage;
use usd_model::{CanonicalValue, SemanticInfo, SemanticProperty};

use crate::config::SemanticConfig;

pub fn extract_metadata(
    stage: &Stage,
    path: &Path,
    config: &SemanticConfig,
) -> Result<(SemanticInfo, Vec<SemanticProperty>)> {
    let prim = stage.prim(path.clone());
    let type_name = prim.type_name()?.map(|value| value.as_str().to_owned());
    let category = prim.kind()?.map(|value| value.as_str().to_owned());
    let display_name = match config.display_name_property.as_deref() {
        Some(property) => configured_text_property(stage, path, Some(property))?,
        None => authored_display_name(stage, path)?,
    };

    let mut properties = if config.include_custom_properties {
        extract_custom_properties(stage, path)?
    } else {
        Vec::new()
    };
    crate::nvidia::attach_measurements(&mut properties, &config.nvidia_revit);
    properties.sort_by(|left, right| left.name.cmp(&right.name));

    let family = configured_text_from_properties(&properties, config.family_property.as_deref());
    let type_id = configured_text_from_properties(&properties, config.type_id_property.as_deref());

    Ok((
        SemanticInfo {
            category,
            family,
            type_name,
            type_id,
            display_name,
        },
        properties,
    ))
}

fn authored_display_name(stage: &Stage, path: &Path) -> Result<Option<String>> {
    let Some(scene_graph) = SceneGraphPrimAPI::get(stage, path.clone())? else {
        return Ok(None);
    };
    Ok(scene_graph
        .display_name_attr()
        .get::<Token>()?
        .map(|value| value.as_str().to_owned()))
}

fn extract_custom_properties(stage: &Stage, path: &Path) -> Result<Vec<SemanticProperty>> {
    let prim = stage.prim(path.clone());
    let mut properties = Vec::new();
    for attribute in prim.attributes()? {
        if !attribute.is_custom()? {
            continue;
        }
        let Some(value) = attribute.get::<Value>()? else {
            continue;
        };
        let name = attribute
            .path()
            .as_str()
            .split_once('.')
            .map(|(_, property)| property.to_owned())
            .unwrap_or_else(|| attribute.path().as_str().to_owned());
        properties.push(SemanticProperty {
            name,
            value: canonical_value(value),
            measurement: None,
        });
    }
    Ok(properties)
}

fn configured_text_property(
    stage: &Stage,
    path: &Path,
    property: Option<&str>,
) -> Result<Option<String>> {
    let Some(property) = property else {
        return Ok(None);
    };
    let value = stage
        .prim(path.clone())
        .attribute(property)
        .get::<Value>()?;
    Ok(value.and_then(text_value))
}

fn configured_text_from_properties(
    properties: &[SemanticProperty],
    property: Option<&str>,
) -> Option<String> {
    let property = property?;
    properties
        .iter()
        .find(|candidate| candidate.name == property)
        .and_then(|candidate| match &candidate.value {
            CanonicalValue::Text(value) => Some(value.clone()),
            _ => None,
        })
}

fn text_value(value: Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value),
        Value::Token(value) => Some(value.as_str().to_owned()),
        Value::AssetPath(value) => Some(value.as_str().to_owned()),
        _ => None,
    }
}

pub(crate) fn canonical_value(value: Value) -> CanonicalValue {
    match value {
        Value::None | Value::ValueBlock => CanonicalValue::Null,
        Value::Bool(value) => CanonicalValue::Bool(value),
        Value::Uchar(value) => CanonicalValue::Integer(i64::from(value)),
        Value::Int(value) => CanonicalValue::Integer(i64::from(value)),
        Value::Uint(value) => CanonicalValue::Integer(i64::from(value)),
        Value::Int64(value) => CanonicalValue::Integer(value),
        Value::Uint64(value) => match i64::try_from(value) {
            Ok(value) => CanonicalValue::Integer(value),
            Err(_) => CanonicalValue::Json(value.to_string()),
        },
        Value::Half(value) => CanonicalValue::Real(f64::from(f32::from(value))),
        Value::Float(value) => CanonicalValue::Real(f64::from(value)),
        Value::Double(value) => CanonicalValue::Real(value),
        Value::String(value) => CanonicalValue::Text(value),
        Value::Token(value) => CanonicalValue::Text(value.as_str().to_owned()),
        Value::AssetPath(value) => CanonicalValue::Text(value.as_str().to_owned()),
        Value::StringVec(values) => CanonicalValue::TextArray(values),
        Value::TokenVec(values) => CanonicalValue::TextArray(
            values
                .into_iter()
                .map(|value| value.as_str().to_owned())
                .collect(),
        ),
        Value::AssetPathVec(values) => CanonicalValue::TextArray(
            values
                .into_iter()
                .map(|value| value.as_str().to_owned())
                .collect(),
        ),
        Value::PathVec(values) => CanonicalValue::TextArray(
            values
                .into_iter()
                .map(|value| value.as_str().to_owned())
                .collect(),
        ),
        Value::BoolVec(values) => CanonicalValue::Json(format!("{values:?}")),
        Value::UcharVec(values) => number_array(values.into_iter().map(f64::from)),
        Value::IntVec(values) => number_array(values.into_iter().map(f64::from)),
        Value::UintVec(values) => number_array(values.into_iter().map(f64::from)),
        Value::Int64Vec(values) => number_array(values.into_iter().map(|value| value as f64)),
        Value::Uint64Vec(values) => number_array(values.into_iter().map(|value| value as f64)),
        Value::HalfVec(values) => {
            number_array(values.into_iter().map(|value| f64::from(f32::from(value))))
        }
        Value::FloatVec(values) => number_array(values.into_iter().map(f64::from)),
        Value::DoubleVec(values) => CanonicalValue::NumberArray(values),
        Value::Vec2h(value) => number_array(
            [value.x, value.y]
                .into_iter()
                .map(|value| f64::from(f32::from(value))),
        ),
        Value::Vec2f(value) => number_array([value.x, value.y].into_iter().map(f64::from)),
        Value::Vec2d(value) => number_array([value.x, value.y]),
        Value::Vec2i(value) => number_array([value.x, value.y].into_iter().map(f64::from)),
        Value::Vec3h(value) => number_array(
            [value.x, value.y, value.z]
                .into_iter()
                .map(|value| f64::from(f32::from(value))),
        ),
        Value::Vec3f(value) => number_array([value.x, value.y, value.z].into_iter().map(f64::from)),
        Value::Vec3d(value) => number_array([value.x, value.y, value.z]),
        Value::Vec3i(value) => number_array([value.x, value.y, value.z].into_iter().map(f64::from)),
        Value::Vec4h(value) => number_array(
            [value.x, value.y, value.z, value.w]
                .into_iter()
                .map(|value| f64::from(f32::from(value))),
        ),
        Value::Vec4f(value) => number_array(
            [value.x, value.y, value.z, value.w]
                .into_iter()
                .map(f64::from),
        ),
        Value::Vec4d(value) => number_array([value.x, value.y, value.z, value.w]),
        Value::Vec4i(value) => number_array(
            [value.x, value.y, value.z, value.w]
                .into_iter()
                .map(f64::from),
        ),
        Value::Quath(value) => number_array(
            [value.w, value.x, value.y, value.z]
                .into_iter()
                .map(|value| f64::from(f32::from(value))),
        ),
        Value::Quatf(value) => number_array(
            [value.w, value.x, value.y, value.z]
                .into_iter()
                .map(f64::from),
        ),
        Value::Quatd(value) => number_array([value.w, value.x, value.y, value.z]),
        Value::Matrix4d(value) => CanonicalValue::NumberArray(value.0.to_vec()),
        Value::Dictionary(value) => CanonicalValue::Json(stable_dictionary(&value)),
        other => CanonicalValue::Json(format!("{other:?}")),
    }
}

fn number_array(values: impl IntoIterator<Item = f64>) -> CanonicalValue {
    CanonicalValue::NumberArray(values.into_iter().collect())
}

fn stable_dictionary(values: &std::collections::HashMap<String, Value>) -> String {
    let mut entries: Vec<_> = values.iter().collect();
    entries.sort_by(|left, right| left.0.cmp(right.0));
    let mut output = String::from("{");
    for (index, (name, value)) in entries.into_iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str(name);
        output.push('=');
        output.push_str(&stable_value(value));
    }
    output.push('}');
    output
}

fn stable_value(value: &Value) -> String {
    match value {
        Value::Dictionary(values) => stable_dictionary(values),
        _ => format!("{value:?}"),
    }
}
