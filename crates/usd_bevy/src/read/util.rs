//! Shared attribute/relationship plumbing for the `read` modules.
//!
//! These helpers read composed values straight off openusd's public
//! `Prim` / `Attribute` / `Relationship` handles — the crate reads the
//! authored scene through openusd only, with no schema layer in between.
//! Each helper takes the owning prim path plus the property name and
//! returns the decoded value (`None` when unauthored or a type mismatch),
//! keeping schema decoding local to the current openusd-backed reader layer.

use openusd::sdf::Path;
use openusd::usd::Stage;
use openusd::vt::{CastError, Value, ValueKind};

/// Raw composed `default` value of attribute `name` on `prim`.
fn attr_default(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<Value>> {
    Ok(stage.prim(prim.clone())?.attribute(name).get::<Value>()?)
}

pub fn read_f32(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<f32>> {
    Ok(attr_default(stage, prim, name)?.and_then(|v| v.cast::<f32>().ok()))
}

pub fn read_double(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<f64>> {
    Ok(attr_default(stage, prim, name)?.and_then(|v| v.cast::<f64>().ok()))
}

/// A `double`, `float`, or `timecode` scalar as `f64`. (openusd decodes
/// `timecode` into a `Double`, so the two collapse here.)
pub fn read_double_or_timecode(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<f64>> {
    Ok(attr_default(stage, prim, name)?
        .filter(|v| matches!(ValueKind::from(v), ValueKind::Double | ValueKind::Float | ValueKind::Half | ValueKind::TimeCode))
        .and_then(|v| v.cast::<f64>().ok()))
}

pub fn read_int(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<i32>> {
    match attr_default(stage, prim, name)? {
        None => Ok(None),
        Some(Value::Int(i)) => Ok(Some(i)),
        Some(Value::Int64(i)) => i32::try_from(i).map(Some).map_err(|_| {
            anyhow::anyhow!(
                "authored int64 value on {prim}.{name} is outside representable i32 range"
            )
        }),
        Some(other) => match other.cast::<i32>() {
            Ok(i) => Ok(Some(i)),
            Err(CastError::OutOfRange { .. }) => Err(anyhow::anyhow!(
                "authored numeric value on {prim}.{name} is outside representable i32 range"
            )),
            Err(CastError::TypeMismatch { .. }) => Ok(None),
        },
    }
}

pub fn read_bool(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<bool>> {
    Ok(attr_default(stage, prim, name)?.and_then(|v| v.cast::<bool>().ok()))
}

/// A `token` or `string` scalar.
pub fn read_token_or_string(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<String>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Token(s)) => Some(s.as_str().to_string()),
        Some(Value::String(s)) => Some(s),
        _ => None,
    })
}

/// Decode a metadata text value, including USDA `Value::UnregisteredValue`
/// string literals produced for unregistered fields such as `ui:displayName`.
pub fn decode_metadata_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Token(token) => Some(token.as_str().to_owned()),
        Value::UnregisteredValue(literal) => decode_unregistered_string_literal(literal),
        _ => None,
    }
}

fn decode_unregistered_string_literal(literal: &str) -> Option<String> {
    let trimmed = literal.trim();
    for quote in ["\"\"\"", "'''", "\"", "'"] {
        if trimmed.len() >= quote.len() * 2
            && trimmed.starts_with(quote)
            && trimmed.ends_with(quote)
        {
            let body = &trimmed[quote.len()..trimmed.len() - quote.len()];
            return Some(unescape_usda_string(body));
        }
    }
    // `usdc` converts unregistered string fields directly into `UnregisteredValue(text)`
    // without surrounding quotes, whereas non-string USDA literals (numbers, lists, tuples,
    // dictionaries, `None`, booleans) must not be treated as strings.
    if trimmed == "None"
        || trimmed == "true"
        || trimmed == "false"
        || trimmed.starts_with('[')
        || trimmed.starts_with('(')
        || trimmed.starts_with('{')
        || trimmed.starts_with('@')
        || trimmed.parse::<f64>().is_ok()
    {
        return None;
    }
    Some(literal.to_owned())
}

fn unescape_usda_string(body: &str) -> String {
    if !body.contains('\\') {
        return body.to_owned();
    }
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(escaped) = chars.next() else {
            break;
        };
        match escaped {
            'a' => out.push('\u{7}'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'v' => out.push('\u{b}'),
            'x' => {
                let mut code = 0u32;
                let mut digits = 0;
                while digits < 2 {
                    let Some(digit) = chars.peek().and_then(|ch| ch.to_digit(16)) else {
                        break;
                    };
                    chars.next();
                    code = code * 16 + digit;
                    digits += 1;
                }
                if digits > 0
                    && let Some(ch) = char::from_u32(code)
                {
                    out.push(ch);
                }
            }
            '0'..='7' => {
                let mut code = escaped.to_digit(8).unwrap_or_default();
                for _ in 1..3 {
                    let Some(digit) = chars.peek().and_then(|ch| ch.to_digit(8)) else {
                        break;
                    };
                    chars.next();
                    code = code * 8 + digit;
                }
                if let Some(ch) = char::from_u32(code) {
                    out.push(ch);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// An `asset`, `string`, or `token` scalar (asset path or plain text).
pub fn read_asset_path(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<String>> {
    Ok(attr_default(stage, prim, name)?.and_then(|v| v.as_str().map(str::to_owned)))
}

pub fn read_vec3f(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<[f32; 3]>> {
    Ok(attr_default(stage, prim, name)?.and_then(|v| v.cast::<openusd::gf::Vec3f>().ok().map(Into::into)))
}

pub fn read_vec2f(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<[f32; 2]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec2f(v)) => Some(v.into()),
        Some(Value::Vec2d(v)) => Some([v.x as f32, v.y as f32]),
        Some(Value::Vec2h(v)) => Some([f32::from(v.x), f32::from(v.y)]),
        _ => None,
    })
}

pub fn read_vec2i(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<[i32; 2]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec2i(v)) => Some(v.into()),
        Some(Value::IntVec(v)) if v.len() == 2 => Some([v[0], v[1]]),
        _ => None,
    })
}

pub fn read_token_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<String>> {
    Ok(attr_default(stage, prim, name)?
        .and_then(|v| v.cast::<Vec<String>>().ok())
        .unwrap_or_default())
}

pub fn read_int_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<i32>> {
    match attr_default(stage, prim, name)? {
        Some(Value::IntVec(v)) => Ok(v),
        Some(Value::Int64Vec(v)) => v
            .into_iter()
            .map(|i| {
                i32::try_from(i).map_err(|_| {
                    anyhow::anyhow!(
                        "authored int64[] element on {prim}.{name} is outside representable i32 range"
                    )
                })
            })
            .collect(),
        _ => Ok(Vec::new()),
    }
}

pub fn read_float_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<f32>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::FloatVec(v)) => v,
        Some(Value::DoubleVec(v)) => v.into_iter().map(|d| d as f32).collect(),
        Some(other) => other.cast::<Vec<f32>>().unwrap_or_default(),
        None => Vec::new(),
    })
}

pub fn read_double_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<f64>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::DoubleVec(v)) => v,
        Some(Value::FloatVec(v)) => v.into_iter().map(|f| f as f64).collect(),
        _ => Vec::new(),
    })
}

pub fn read_vec3f_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<[f32; 3]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec3fVec(v)) => v.into_iter().map(Into::into).collect(),
        _ => Vec::new(),
    })
}

pub fn read_vec2f_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<[f32; 2]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec2fVec(v)) => v.into_iter().map(Into::into).collect(),
        _ => Vec::new(),
    })
}

pub fn read_vec2d_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<[f64; 2]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec2dVec(v)) => v.into_iter().map(Into::into).collect(),
        _ => Vec::new(),
    })
}

pub fn read_quatf_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<[f32; 4]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::QuatfVec(v)) => v.into_iter().map(Into::into).collect(),
        _ => Vec::new(),
    })
}

/// Like [`read_int_vec`] but distinguishes "unauthored" (`None`) from "empty".
pub fn read_int_vec_opt(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Vec<i32>>> {
    match attr_default(stage, prim, name)? {
        Some(Value::IntVec(v)) => Ok(Some(v)),
        Some(Value::Int64Vec(v)) => v
            .into_iter()
            .map(|i| {
                i32::try_from(i).map_err(|_| {
                    anyhow::anyhow!(
                        "authored int64[] element on {prim}.{name} is outside representable i32 range"
                    )
                })
            })
            .collect::<anyhow::Result<Vec<i32>>>()
            .map(Some),
        _ => Ok(None),
    }
}

/// Like [`read_float_vec`] but distinguishes "unauthored" (`None`) from "empty".
pub fn read_float_vec_opt(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Vec<f32>>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::FloatVec(v)) => Some(v),
        Some(Value::DoubleVec(v)) => Some(v.into_iter().map(|d| d as f32).collect()),
        _ => None,
    })
}

/// `matrix4d[]` flattened to row-major `[f32; 16]` per element.
pub fn read_mat4f_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<[f32; 16]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Matrix4dVec(v)) => v
            .into_iter()
            .map(|m| {
                let mut out = [0.0f32; 16];
                for (i, slot) in out.iter_mut().enumerate() {
                    *slot = m.0[i] as f32;
                }
                out
            })
            .collect(),
        _ => Vec::new(),
    })
}

/// `i32`-valued attribute *metadata* field (e.g. primvar `elementSize`).
pub fn read_int_metadata(
    stage: &Stage,
    prim: &Path,
    attr: &str,
    key: &str,
) -> anyhow::Result<Option<i32>> {
    match stage
        .prim(prim.clone())?
        .attribute(attr)
        .metadata::<Value>(key)?
    {
        Some(Value::Int(n)) => Ok(Some(n)),
        Some(Value::Int64(n)) => i32::try_from(n).map(Some).map_err(|_| {
            anyhow::anyhow!(
                "authored int64 metadata {key} on {prim}.{attr} is outside representable i32 range"
            )
        }),
        _ => Ok(None),
    }
}

/// Composed `timeSamples` for an attribute, as `(time, value)` pairs.
pub fn read_time_samples(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Vec<(f64, Value)>> {
    Ok(stage
        .prim(prim.clone())?
        .attribute(name)
        .time_samples()?
        .unwrap_or_default())
}

/// Composed relationship target paths (as strings), in authored order.
pub fn read_rel_targets(stage: &Stage, prim: &Path, rel_name: &str) -> anyhow::Result<Vec<String>> {
    let targets = stage.prim(prim.clone())?.relationship(rel_name).targets()?;
    Ok(targets
        .into_iter()
        .map(|p| p.as_str().to_string())
        .collect())
}

/// The first composed relationship target (strongest), if any.
pub fn read_rel_first_target(
    stage: &Stage,
    prim: &Path,
    rel_name: &str,
) -> anyhow::Result<Option<String>> {
    Ok(read_rel_targets(stage, prim, rel_name)?.into_iter().next())
}

// ── Property-path-based access (for graph walks like UsdShade) ──────────

/// Composed `connectionPaths` of the attribute at property path `attr_path`.
pub fn connections_at(stage: &Stage, attr_path: &Path) -> anyhow::Result<Vec<Path>> {
    let Some((prim, name)) = attr_path.split_property() else {
        return Ok(Vec::new());
    };
    Ok(stage.prim(prim)?.attribute(name).connections()?)
}

/// Composed relationship target paths at property path `rel_path`.
pub fn targets_at(stage: &Stage, rel_path: &Path) -> anyhow::Result<Vec<Path>> {
    let Some((prim, name)) = rel_path.split_property() else {
        return Ok(Vec::new());
    };
    Ok(stage.prim(prim)?.relationship(name).targets()?)
}

/// Raw composed `default` value of the attribute at property path `attr_path`.
pub fn default_at(stage: &Stage, attr_path: &Path) -> anyhow::Result<Option<Value>> {
    let Some((prim, name)) = attr_path.split_property() else {
        return Ok(None);
    };
    Ok(stage.prim(prim)?.attribute(name).get::<Value>()?)
}
