//! Author-back (PLAN P2): ECS component → USD opinions.
//!
//! The write direction of the reflect route. Where [`crate::route::reflect`]
//! projects `bevy:` attributes onto components, this reads a component back
//! and authors its fields as `bevy:<Type>:<field>` attributes into the stage's
//! current edit target. This is the round-trip BSN structurally can't do:
//! spawned entities remain views of the stage, and edits flow back into
//! layers that persist and export.
//!
//! USD is still the source of truth. After authoring, the commit fires the
//! change sink and the reflect route re-projects — writing the same value it
//! just read, so the round-trip is idempotent (see the echo guard on
//! [`crate::live::LiveStage`]).

use bevy::ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy::prelude::*;
use bevy::reflect::enums::VariantType;
use bevy::reflect::{PartialReflect, ReflectRef, TypeRegistry};
use openusd::sdf::Value;
use openusd::usd::Stage;

use crate::authoring::set_attribute;

type Result<T> = anyhow::Result<T>;

/// A reflected leaf value as a USD `(value, typeName)` pair, or `None` for a
/// field type we don't author (mirrors the forward route's coverage).
fn value_of(field: &dyn PartialReflect) -> Option<(Value, &'static str)> {
    if let Some(x) = field.try_downcast_ref::<f64>() {
        return Some((Value::Double(*x), "double"));
    }
    if let Some(x) = field.try_downcast_ref::<f32>() {
        return Some((Value::Float(*x), "float"));
    }
    if let Some(x) = field.try_downcast_ref::<i32>() {
        return Some((Value::Int(*x), "int"));
    }
    if let Some(x) = field.try_downcast_ref::<i64>() {
        return Some((Value::Int64(*x), "int64"));
    }
    if let Some(x) = field.try_downcast_ref::<u32>() {
        return Some((Value::Uint(*x), "uint"));
    }
    if let Some(x) = field.try_downcast_ref::<u64>() {
        return Some((Value::Uint64(*x), "uint64"));
    }
    if let Some(x) = field.try_downcast_ref::<usize>() {
        return Some((Value::Uint64(*x as u64), "uint64"));
    }
    if let Some(x) = field.try_downcast_ref::<bool>() {
        return Some((Value::Bool(*x), "bool"));
    }
    if let Some(x) = field.try_downcast_ref::<String>() {
        return Some((Value::String(x.clone()), "string"));
    }
    if let Some(x) = field.try_downcast_ref::<Vec2>() {
        return Some((Value::Vec2f([x.x, x.y].into()), "float2"));
    }
    if let Some(x) = field.try_downcast_ref::<Vec3>() {
        return Some((Value::Vec3f([x.x, x.y, x.z].into()), "float3"));
    }
    if let Some(x) = field.try_downcast_ref::<Vec4>() {
        return Some((Value::Vec4f([x.x, x.y, x.z, x.w].into()), "float4"));
    }
    if let Some(x) = field.try_downcast_ref::<Quat>() {
        // gf Quatf fields are (w, x, y, z); build them explicitly rather than
        // via `From<[f32;4]>` (which is real-first and would scramble xyzw).
        return Some((
            Value::Quatf(openusd::gf::Quatf {
                w: x.w,
                x: x.x,
                y: x.y,
                z: x.z,
            }),
            "quatf",
        ));
    }
    if let Some(c) = field.try_downcast_ref::<Color>() {
        // Author as linear `color4f` (round-trips with the forward route's
        // linear mapping, preserving alpha).
        let l = c.to_linear();
        return Some((
            Value::Vec4f([l.red, l.green, l.blue, l.alpha].into()),
            "color4f",
        ));
    }
    // Array fields.
    if let Some(x) = field.try_downcast_ref::<Vec<f32>>() {
        return Some((Value::FloatVec(x.clone()), "float[]"));
    }
    if let Some(x) = field.try_downcast_ref::<Vec<i32>>() {
        return Some((Value::IntVec(x.clone()), "int[]"));
    }
    if let Some(x) = field.try_downcast_ref::<Vec<String>>() {
        return Some((Value::StringVec(x.clone()), "string[]"));
    }
    if let Some(x) = field.try_downcast_ref::<Vec<Vec3>>() {
        let v = x.iter().map(|p| [p.x, p.y, p.z].into()).collect();
        return Some((Value::Vec3fVec(v), "float3[]"));
    }
    // Option<T>: `Some` authors the inner value; `None` authors nothing.
    if let Some(o) = field.try_downcast_ref::<Option<f32>>() {
        return (*o).map(|x| (Value::Float(x), "float"));
    }
    if let Some(o) = field.try_downcast_ref::<Option<f64>>() {
        return (*o).map(|x| (Value::Double(x), "double"));
    }
    if let Some(o) = field.try_downcast_ref::<Option<i32>>() {
        return (*o).map(|x| (Value::Int(x), "int"));
    }
    if let Some(o) = field.try_downcast_ref::<Option<bool>>() {
        return (*o).map(|x| (Value::Bool(x), "bool"));
    }
    if let Some(o) = field.try_downcast_ref::<Option<String>>() {
        return o.as_ref().map(|x| (Value::String(x.clone()), "string"));
    }
    if let Some(o) = field.try_downcast_ref::<Option<Vec3>>() {
        return o.map(|v| (Value::Vec3f([v.x, v.y, v.z].into()), "float3"));
    }
    // Unit-variant enums author as a token naming the variant.
    if let ReflectRef::Enum(e) = field.reflect_ref()
        && e.variant_type() == VariantType::Unit
    {
        return Some((Value::Token(e.variant_name().into()), "token"));
    }
    None
}

/// Walk a reflected value into leaf `(field_path, value, typeName)` triples,
/// descending named struct fields with `:` (the same namespacing the forward
/// route reads). A leaf is any value [`value_of`] can encode; a whole vector /
/// quaternion is one leaf (tried before descending its `x`/`y`/`z` fields).
fn walk(field: &dyn PartialReflect, prefix: &str, out: &mut Vec<(String, Value, &'static str)>) {
    if let Some((v, ty)) = value_of(field) {
        out.push((prefix.to_string(), v, ty));
        return;
    }
    match field.reflect_ref() {
        ReflectRef::Struct(s) => {
            for i in 0..s.field_len() {
                let Some(name) = s.name_at(i) else { continue };
                let Some(child) = s.field_at(i) else { continue };
                let path = if prefix.is_empty() {
                    name.to_string()
                } else {
                    format!("{prefix}:{name}")
                };
                walk(child, &path, out);
            }
        }
        // Tuple structs (incl. newtypes like `struct Level(u32)`) descend by
        // index. USD identifiers can't start with a digit, so the index is
        // authored as `_0`, `_1`, … (decoded back to the reflect path `.0` by
        // the forward route).
        ReflectRef::TupleStruct(ts) => {
            for i in 0..ts.field_len() {
                let Some(child) = ts.field(i) else { continue };
                let path = if prefix.is_empty() {
                    format!("_{i}")
                } else {
                    format!("{prefix}:_{i}")
                };
                walk(child, &path, out);
            }
        }
        // Data-carrying enum variants (PLAN 4c): author the variant name at this
        // path (mirroring the unit case in `value_of`), then descend the active
        // variant's payload — tuple fields as `_i`, struct fields by name — so
        // the forward route reconstructs the variant. (Unit variants never reach
        // here: `value_of` already emitted them as a token.)
        ReflectRef::Enum(e) => {
            // `Option<T>` is a reflect enum too; `value_of` owns it (a `Some`
            // authors its inner value, a `None` authors nothing). Never treat it
            // as a variant token, or a `None` field would leak a `"None"` token.
            if field
                .reflect_type_path()
                .starts_with("core::option::Option")
            {
                return;
            }
            out.push((
                prefix.to_string(),
                Value::Token(e.variant_name().into()),
                "token",
            ));
            match e.variant_type() {
                VariantType::Tuple => {
                    for i in 0..e.field_len() {
                        let Some(child) = e.field_at(i) else { continue };
                        let path = if prefix.is_empty() {
                            format!("_{i}")
                        } else {
                            format!("{prefix}:_{i}")
                        };
                        walk(child, &path, out);
                    }
                }
                VariantType::Struct => {
                    for i in 0..e.field_len() {
                        let Some(name) = e.name_at(i) else { continue };
                        let Some(child) = e.field_at(i) else { continue };
                        let path = if prefix.is_empty() {
                            name.to_string()
                        } else {
                            format!("{prefix}:{name}")
                        };
                        walk(child, &path, out);
                    }
                }
                VariantType::Unit => {}
            }
        }
        _ => {}
    }
}

/// Author every encodable field of the reflect component `short_type` on
/// `entity` as `bevy:<short_type>:<field>` attributes on `prim_path`, into the
/// stage's current edit target. Returns the authored attribute names.
///
/// This is the inverse of the reflect route. Pair it with the stage's edit
/// target (session layer for a scratch override, a stronger layer to persist)
/// and, in a live session, the echo guard so the re-projection is swallowed.
pub fn author_component(
    world: &World,
    registry: &TypeRegistry,
    stage: &Stage,
    entity: Entity,
    prim_path: &str,
    short_type: &str,
) -> Result<Vec<String>> {
    let registration = registry
        .get_with_short_type_path(short_type)
        .or_else(|| registry.get_with_type_path(short_type))
        .ok_or_else(|| anyhow::anyhow!("type `{short_type}` is not registered"))?;
    let reflect_component = registration
        .data::<ReflectComponent>()
        .ok_or_else(|| anyhow::anyhow!("type `{short_type}` has no ReflectComponent"))?;
    let entity_ref = world
        .get_entity(entity)
        .map_err(|_| anyhow::anyhow!("entity {entity:?} does not exist"))?;
    let component = reflect_component
        .reflect(entity_ref)
        .ok_or_else(|| anyhow::anyhow!("entity {entity:?} has no `{short_type}` component"))?;

    let mut leaves = Vec::new();
    walk(component.as_partial_reflect(), "", &mut leaves);

    let mut authored = Vec::new();
    for (path, value, type_name) in leaves {
        let attr = format!("bevy:{short_type}:{path}");
        set_attribute(stage, prim_path, &attr, type_name, value)?;
        authored.push(attr);
    }
    Ok(authored)
}

/// Convenience for reading the `AppTypeRegistry` out of the world and authoring
/// `short_type` on `entity` in one call. The registry read-lock is held only
/// for the duration of the author.
pub fn author_component_from_world(
    world: &World,
    stage: &Stage,
    entity: Entity,
    prim_path: &str,
    short_type: &str,
) -> Result<Vec<String>> {
    let app_registry = world
        .get_resource::<AppTypeRegistry>()
        .ok_or_else(|| anyhow::anyhow!("no AppTypeRegistry in world"))?
        .clone();
    let registry = app_registry.read();
    author_component(world, &registry, stage, entity, prim_path, short_type)
}

#[cfg(test)]
mod tests;
