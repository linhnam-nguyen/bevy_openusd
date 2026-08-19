use openusd::sdf::{Path, Value};
use openusd::usd::Stage;

use super::mesh::Interpolation;

// ── Low-level attribute plumbing ─────────────────────────────────────────

/// Composed `default` value, falling back to the first time sample when the
/// default is an empty array placeholder (common in FX caches).
pub(super) fn attr_default(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Value>> {
    let attr = stage.prim(prim.clone()).attribute(name);
    if let Some(v) = attr.get::<Value>()?
        && !is_empty_array_value(&v)
    {
        return Ok(Some(v));
    }
    Ok(attr
        .time_samples()?
        .unwrap_or_default()
        .into_iter()
        .next()
        .map(|(_, v)| v))
}

fn is_empty_array_value(v: &Value) -> bool {
    match v {
        Value::IntVec(a) => a.is_empty(),
        Value::FloatVec(a) => a.is_empty(),
        Value::DoubleVec(a) => a.is_empty(),
        Value::TokenVec(a) => a.is_empty(),
        Value::StringVec(a) => a.is_empty(),
        Value::Vec2fVec(a) => a.is_empty(),
        Value::Vec2dVec(a) => a.is_empty(),
        Value::Vec3fVec(a) => a.is_empty(),
        Value::Vec3dVec(a) => a.is_empty(),
        Value::Vec4fVec(a) => a.is_empty(),
        _ => false,
    }
}

pub(super) fn read_token(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<String>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Token(s)) => Some(s.as_str().to_string()),
        Some(Value::String(s)) => Some(s),
        _ => None,
    })
}

pub(super) fn read_double(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<f64>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Double(v)) => Some(v),
        Some(Value::Float(v)) => Some(v as f64),
        _ => None,
    })
}

pub(super) fn read_bool(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<bool>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Bool(b)) => Some(b),
        _ => None,
    })
}

pub(super) fn read_extent(stage: &Stage, prim: &Path) -> anyhow::Result<Option<[[f32; 3]; 2]>> {
    Ok(match read_vec3f_array(stage, prim, "extent")? {
        Some(v) if v.len() >= 2 => Some([v[0], v[1]]),
        _ => None,
    })
}

pub(super) fn read_int_array(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Vec<i32>>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::IntVec(v)) => Some(v),
        _ => None,
    })
}

pub(super) fn read_double_array(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Vec<f64>>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::DoubleVec(v)) => Some(v),
        Some(Value::FloatVec(v)) => Some(v.into_iter().map(|f| f as f64).collect()),
        _ => None,
    })
}

pub(super) fn read_int_scalar(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<i32>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Int(v)) => Some(v),
        _ => None,
    })
}

pub(super) fn read_float_array(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Vec<f32>>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::FloatVec(v)) => Some(v),
        Some(Value::DoubleVec(v)) => Some(v.into_iter().map(|d| d as f32).collect()),
        Some(Value::Float(v)) => Some(vec![v]),
        _ => None,
    })
}

pub(super) fn read_int64_array(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Vec<i64>>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Int64Vec(v)) => Some(v),
        Some(Value::IntVec(v)) => Some(v.into_iter().map(|i| i as i64).collect()),
        _ => None,
    })
}

pub(super) fn read_quat_array(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Vec<[f32; 4]>>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::QuatfVec(v)) => Some(v.into_iter().map(|q| [q.w, q.x, q.y, q.z]).collect()),
        Some(Value::QuatdVec(v)) => Some(
            v.into_iter()
                .map(|q| [q.w as f32, q.x as f32, q.y as f32, q.z as f32])
                .collect(),
        ),
        _ => None,
    })
}

pub(super) fn read_vec2d_scalar(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<[f64; 2]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec2d(v)) => Some([v.x, v.y]),
        Some(Value::Vec2f(v)) => Some([v.x as f64, v.y as f64]),
        _ => None,
    })
}

pub(super) fn read_vec2d_array(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Vec<[f64; 2]>>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec2dVec(v)) => Some(v.into_iter().map(|a| [a.x, a.y]).collect()),
        Some(Value::Vec2fVec(v)) => Some(v.into_iter().map(|a| [a.x as f64, a.y as f64]).collect()),
        _ => None,
    })
}

pub(super) fn read_vec3f_array(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Vec<[f32; 3]>>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec3fVec(v)) => Some(v.into_iter().map(|a| [a.x, a.y, a.z]).collect()),
        Some(Value::Vec3dVec(v)) => Some(
            v.into_iter()
                .map(|a| [a.x as f32, a.y as f32, a.z as f32])
                .collect(),
        ),
        _ => None,
    })
}

pub(super) fn read_vec2f_array(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Vec<[f32; 2]>>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec2fVec(v)) => Some(v.into_iter().map(|a| [a.x, a.y]).collect()),
        Some(Value::Vec2dVec(v)) => Some(v.into_iter().map(|a| [a.x as f32, a.y as f32]).collect()),
        _ => None,
    })
}

pub(super) fn read_primvar_interpolation(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Interpolation>> {
    let raw = stage
        .prim(prim.clone())
        .attribute(name)
        .get_metadata::<Value>("interpolation")?;
    if let Some(s) = raw.and_then(|v| match v {
        Value::Token(t) => Some(t.as_str().to_string()),
        Value::String(s) => Some(s),
        _ => None,
    }) && let Some(i) = Interpolation::parse(&s)
    {
        return Ok(Some(i));
    }
    let fallback_name = format!("{name}:interpolation");
    Ok(read_token(stage, prim, &fallback_name)?
        .as_deref()
        .and_then(Interpolation::parse))
}
