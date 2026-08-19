use bevy::image::Image;
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;
use bevy::reflect::enums::{DynamicEnum, DynamicVariant, VariantInfo};
use bevy::reflect::structs::DynamicStruct;
use bevy::reflect::tuple::DynamicTuple;
use bevy::reflect::{PartialReflect, ReflectRef, TypeInfo};
use openusd::sdf::Value;

use super::parse::{as_f64, as_i64, as_vec};

/// Coerce a USD value into a reflected field, matching on the field's concrete
/// Rust type (numeric widths coerce; glam vectors/quat map component-wise).
/// Returns whether the field was set.
pub(super) fn set_field(field: &mut dyn PartialReflect, v: &Value) -> bool {
    // Scalars ------------------------------------------------------------
    if let Some(f) = field.try_downcast_mut::<f32>()
        && let Some(n) = as_f64(v)
    {
        *f = n as f32;
        return true;
    }
    if let Some(f) = field.try_downcast_mut::<f64>()
        && let Some(n) = as_f64(v)
    {
        *f = n;
        return true;
    }
    if let Some(f) = field.try_downcast_mut::<i32>()
        && let Some(n) = as_i64(v)
    {
        *f = n as i32;
        return true;
    }
    if let Some(f) = field.try_downcast_mut::<u32>()
        && let Some(n) = as_i64(v)
    {
        *f = n as u32;
        return true;
    }
    if let Some(f) = field.try_downcast_mut::<i64>()
        && let Some(n) = as_i64(v)
    {
        *f = n;
        return true;
    }
    if let Some(f) = field.try_downcast_mut::<u64>()
        && let Some(n) = as_i64(v)
    {
        *f = n as u64;
        return true;
    }
    if let Some(f) = field.try_downcast_mut::<usize>()
        && let Some(n) = as_i64(v)
    {
        *f = n as usize;
        return true;
    }
    if let Some(b) = field.try_downcast_mut::<bool>()
        && let Value::Bool(x) = v
    {
        *b = *x;
        return true;
    }
    if let Some(s) = field.try_downcast_mut::<String>() {
        match v {
            Value::String(x) => {
                *s = x.clone();
                return true;
            }
            Value::Token(x) => {
                *s = x.as_str().to_string();
                return true;
            }
            _ => {}
        }
    }
    // Vectors / quaternion ----------------------------------------------
    if let Some(out) = field.try_downcast_mut::<Vec2>()
        && let Some(a) = as_vec(v)
    {
        *out = Vec2::new(a[0], a[1]);
        return true;
    }
    if let Some(out) = field.try_downcast_mut::<Vec3>()
        && let Some(a) = as_vec(v)
    {
        *out = Vec3::new(a[0], a[1], a[2]);
        return true;
    }
    if let Some(out) = field.try_downcast_mut::<Vec4>()
        && let Some(a) = as_vec(v)
    {
        *out = Vec4::new(a[0], a[1], a[2], a[3]);
        return true;
    }
    if let Some(out) = field.try_downcast_mut::<Quat>()
        && let Some(a) = as_vec(v)
    {
        *out = Quat::from_xyzw(a[0], a[1], a[2], a[3]);
        return true;
    }
    // Color: `color3f`/`float3` → opaque, `color4f`/`float4` → rgba. USD colors
    // are linear, so map straight into `LinearRgba` (no sRGB transfer).
    if let Some(out) = field.try_downcast_mut::<Color>() {
        match v {
            Value::Vec3f(_) | Value::Vec3d(_) => {
                if let Some(a) = as_vec(v) {
                    *out = Color::linear_rgb(a[0], a[1], a[2]);
                    return true;
                }
            }
            Value::Vec4f(_) | Value::Vec4d(_) => {
                if let Some(a) = as_vec(v) {
                    *out = Color::linear_rgba(a[0], a[1], a[2], a[3]);
                    return true;
                }
            }
            _ => {}
        }
    }
    // Array fields ↔ USD array values (element types we already handle).
    if let Some(out) = field.try_downcast_mut::<Vec<f32>>() {
        match v {
            Value::FloatVec(a) => {
                *out = a.clone();
                return true;
            }
            Value::DoubleVec(a) => {
                *out = a.iter().map(|x| *x as f32).collect();
                return true;
            }
            _ => {}
        }
    }
    if let Some(out) = field.try_downcast_mut::<Vec<i32>>()
        && let Value::IntVec(a) = v
    {
        *out = a.clone();
        return true;
    }
    if let Some(out) = field.try_downcast_mut::<Vec<String>>() {
        match v {
            Value::StringVec(a) => {
                *out = a.clone();
                return true;
            }
            Value::TokenVec(a) => {
                *out = a.iter().map(|t| t.as_str().to_string()).collect();
                return true;
            }
            _ => {}
        }
    }
    if let Some(out) = field.try_downcast_mut::<Vec<Vec3>>() {
        match v {
            Value::Vec3fVec(a) => {
                *out = a.iter().map(|p| Vec3::new(p.x, p.y, p.z)).collect();
                return true;
            }
            Value::Vec3dVec(a) => {
                *out = a
                    .iter()
                    .map(|p| Vec3::new(p.x as f32, p.y as f32, p.z as f32))
                    .collect();
                return true;
            }
            _ => {}
        }
    }
    // Option<T>: a present opinion sets `Some`; a cleared attribute reverts the
    // field to `Option::default()` (`None`) via the route's revert mechanism.
    if let Some(o) = field.try_downcast_mut::<Option<f32>>()
        && let Some(n) = as_f64(v)
    {
        *o = Some(n as f32);
        return true;
    }
    if let Some(o) = field.try_downcast_mut::<Option<f64>>()
        && let Some(n) = as_f64(v)
    {
        *o = Some(n);
        return true;
    }
    if let Some(o) = field.try_downcast_mut::<Option<i32>>()
        && let Some(n) = as_i64(v)
    {
        *o = Some(n as i32);
        return true;
    }
    if let Some(o) = field.try_downcast_mut::<Option<bool>>()
        && let Value::Bool(b) = v
    {
        *o = Some(*b);
        return true;
    }
    if let Some(o) = field.try_downcast_mut::<Option<String>>() {
        match v {
            Value::String(s) => {
                *o = Some(s.clone());
                return true;
            }
            Value::Token(t) => {
                *o = Some(t.as_str().to_string());
                return true;
            }
            _ => {}
        }
    }
    if let Some(o) = field.try_downcast_mut::<Option<Vec3>>()
        && let Some(a) = as_vec(v)
    {
        *o = Some(Vec3::new(a[0], a[1], a[2]));
        return true;
    }
    // Enums: a token/string names the variant. Unit variants switch directly;
    // tuple/struct (data) variants are constructed with synthesized-default
    // payloads (PLAN 4c) — the payload fields are then filled by the sibling
    // `bevy:T:field:_0` / `bevy:T:field:name` attributes, applied after this.
    if matches!(field.reflect_ref(), ReflectRef::Enum(_)) {
        let name = match v {
            Value::Token(t) => Some(t.as_str().to_string()),
            Value::String(s) => Some(s.clone()),
            _ => None,
        };
        if let Some(name) = name {
            return set_enum_variant(field, &name);
        }
    }
    false
}

/// Switch `field` (an enum) to the named variant. For data variants the payload
/// is initialised to synthesized defaults; the caller's per-field pass then
/// overwrites those from the authored payload attributes. Returns whether the
/// variant switch applied.
fn set_enum_variant(field: &mut dyn PartialReflect, variant: &str) -> bool {
    let Some(TypeInfo::Enum(info)) = field.get_represented_type_info() else {
        // No static type info (a dynamic enum): assume a unit variant.
        let dynamic = DynamicEnum::new(variant.to_string(), DynamicVariant::Unit);
        return field.try_apply(&dynamic).is_ok();
    };
    let Some(vinfo) = info.variant(variant) else {
        return false;
    };
    let dv = match vinfo {
        VariantInfo::Unit(_) => DynamicVariant::Unit,
        VariantInfo::Tuple(t) => {
            let mut dt = DynamicTuple::default();
            for i in 0..t.field_len() {
                let Some(def) = synthesize_default(t.field_at(i).unwrap().type_path()) else {
                    return false;
                };
                dt.insert_boxed(def);
            }
            DynamicVariant::Tuple(dt)
        }
        VariantInfo::Struct(s) => {
            let mut ds = DynamicStruct::default();
            for i in 0..s.field_len() {
                let f = s.field_at(i).unwrap();
                let Some(def) = synthesize_default(f.type_path()) else {
                    return false;
                };
                ds.insert_boxed(f.name(), def);
            }
            DynamicVariant::Struct(ds)
        }
    };
    let dynamic = DynamicEnum::new(variant.to_string(), dv);
    field.try_apply(&dynamic).is_ok()
}

/// A zero/empty default value for a variant payload field, by type path. Covers
/// the scalar/string/glam types the reflect route already coerces USD values
/// into; an unsupported payload type makes the whole variant switch fail (and
/// warn), rather than constructing a half-built variant.
fn synthesize_default(type_path: &str) -> Option<Box<dyn PartialReflect>> {
    let v: Box<dyn PartialReflect> = match type_path {
        "f32" => Box::new(0f32),
        "f64" => Box::new(0f64),
        "i32" => Box::new(0i32),
        "u32" => Box::new(0u32),
        "i64" => Box::new(0i64),
        "u64" => Box::new(0u64),
        "usize" => Box::new(0usize),
        "bool" => Box::new(false),
        "alloc::string::String" => Box::new(String::new()),
        "glam::Vec2" => Box::new(Vec2::ZERO),
        "glam::Vec3" => Box::new(Vec3::ZERO),
        "glam::Vec4" => Box::new(Vec4::ZERO),
        _ => return None,
    };
    Some(v)
}

/// Resolve an `asset`/`string` value into a `Handle<T>` field via the
/// `AssetServer` (PLAN 4a). Covers the common asset types; other `Handle<T>`
/// fields fall through to the normal coercion (which won't match, and warns).
/// Path resolution is currently Bevy-asset-root-relative; layer-relative
/// resolution is the openusd-blocked part.
pub(super) fn try_set_handle(
    field: &mut dyn PartialReflect,
    v: &Value,
    assets: Option<&AssetServer>,
) -> bool {
    let Some(server) = assets else {
        return false;
    };
    let path = match v {
        Value::AssetPath(a) => a.as_str().to_string(),
        Value::String(s) => s.clone(),
        Value::Token(t) => t.as_str().to_string(),
        _ => return false,
    };
    if let Some(h) = field.try_downcast_mut::<Handle<Image>>() {
        *h = server.load(path);
        return true;
    }
    if let Some(h) = field.try_downcast_mut::<Handle<Mesh>>() {
        *h = server.load(path);
        return true;
    }
    if let Some(h) = field.try_downcast_mut::<Handle<StandardMaterial>>() {
        *h = server.load(path);
        return true;
    }
    false
}
