use openusd::sdf::Value;

use super::super::RouteCtx;

/// The attribute namespace that marks a reflect-routed component field.
pub(super) const NS: &str = "bevy:";

/// `bevy:` attributes grouped by short type path, each field paired with its
/// composed value (`None` = no effective opinion / blocked).
pub(super) type Groups = Vec<(String, Vec<(String, Option<Value>)>)>;

/// Parse a property name `bevy:<Type>:<field>:<sub>…` into the type segment and
/// a `.`-joined reflect field path. Returns `None` for non-`bevy:` names or a
/// bare `bevy:Type` with no field.
///
/// The type segment is the first `:`-delimited component; it is normally a
/// short type path (`Health`). To disambiguate a short-name collision, author
/// the *full* path with `__` standing in for `::` (`my_game__Health`) — USD
/// rejects a literal `::` in a property name (empty namespace component), so
/// `__` is the path-legal encoding, mapped back in [`resolve`].
pub(super) fn parse_attr(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix(NS)?;
    let mut segs = rest.split(':');
    let ty = segs.next().filter(|s| !s.is_empty())?.to_string();
    let field: Vec<String> = segs.filter(|s| !s.is_empty()).map(decode_index).collect();
    if field.is_empty() {
        return None;
    }
    Some((ty, field.join(".")))
}

/// Decode a tuple-index segment: USD identifiers can't start with a digit, so a
/// tuple field `.0` is authored as `_0` and decoded back here (`_0` → `0`).
/// Named fields (including `_foo`) pass through unchanged.
fn decode_index(seg: &str) -> String {
    if let Some(rest) = seg.strip_prefix('_')
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        rest.to_string()
    } else {
        seg.to_string()
    }
}

/// Resolve a type segment to its registration: short type path first (the
/// common case), then the full type path, then the full path with `__` decoded
/// to `::` (the path-legal way to author a full path for disambiguation).
pub(super) fn resolve<'a>(
    registry: &'a bevy::reflect::TypeRegistry,
    ty: &str,
) -> Option<&'a bevy::reflect::TypeRegistration> {
    registry
        .get_with_short_type_path(ty)
        .or_else(|| registry.get_with_type_path(ty))
        .or_else(|| registry.get_with_type_path(&ty.replace("__", "::")))
}

/// Every authored `bevy:` attribute on the prim, grouped by short type path,
/// each as `(reflect_field_path, composed_value_or_none)`. `None` value means
/// no layer currently authors an opinion (a cleared field).
pub(super) fn collect(ctx: &RouteCtx) -> Groups {
    let prim = ctx.stage.prim(ctx.path.clone());
    let Ok(names) = prim.property_names() else {
        return Vec::new();
    };
    let tc = ctx.time.map(openusd::usd::TimeCode::new);
    let mut groups: Groups = Vec::new();
    for name in names {
        let name = name.as_str();
        let Some((ty, field)) = parse_attr(name) else {
            continue;
        };
        let value = prim.attribute(name).get_at::<Value>(tc).ok().flatten();
        match groups.iter_mut().find(|(t, _)| *t == ty) {
            Some((_, fields)) => fields.push((field, value)),
            None => groups.push((ty, vec![(field, value)])),
        }
    }
    groups
}

pub(super) fn as_f64(v: &Value) -> Option<f64> {
    Some(match v {
        Value::Float(x) => *x as f64,
        Value::Double(x) => *x,
        Value::Half(x) => f32::from(*x) as f64,
        Value::Int(x) => *x as f64,
        Value::Int64(x) => *x as f64,
        Value::Uint(x) => *x as f64,
        Value::Uint64(x) => *x as f64,
        Value::Uchar(x) => *x as f64,
        _ => return None,
    })
}

pub(super) fn as_i64(v: &Value) -> Option<i64> {
    Some(match v {
        Value::Int(x) => *x as i64,
        Value::Int64(x) => *x,
        Value::Uint(x) => *x as i64,
        Value::Uint64(x) => *x as i64,
        Value::Uchar(x) => *x as i64,
        Value::Float(x) => *x as i64,
        Value::Double(x) => *x as i64,
        Value::Bool(x) => *x as i64,
        _ => return None,
    })
}

/// A USD vector/quat value as up to 4 `f32` lanes (missing lanes are 0).
pub(super) fn as_vec(v: &Value) -> Option<[f32; 4]> {
    Some(match v {
        Value::Vec2f(a) => [a.x, a.y, 0.0, 0.0],
        Value::Vec2d(a) => [a.x as f32, a.y as f32, 0.0, 0.0],
        Value::Vec3f(a) => [a.x, a.y, a.z, 0.0],
        Value::Vec3d(a) => [a.x as f32, a.y as f32, a.z as f32, 0.0],
        Value::Vec4f(a) => [a.x, a.y, a.z, a.w],
        Value::Vec4d(a) => [a.x as f32, a.y as f32, a.z as f32, a.w as f32],
        Value::Quatf(q) => [q.x, q.y, q.z, q.w],
        Value::Quatd(q) => [q.x as f32, q.y as f32, q.z as f32, q.w as f32],
        _ => return None,
    })
}
