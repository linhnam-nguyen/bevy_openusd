//! PointInstancer route (PLAN P4, in-repo): `UsdGeomPointInstancer` → one
//! child entity per instance, sharing the prototype's baked mesh.
//!
//! This is *point* instancing (an explicit position/orientation/scale table),
//! distinct from USD native scenegraph instancing (which is openusd-blocked).
//! Instances are spawned as children of the instancer entity, each carrying a
//! [`UsdInstance`] marker so a reproject can clear the previous batch. All
//! instances of one prototype share a single `Mesh`/`StandardMaterial` handle
//! (baked once per project), so N instances cost one mesh in memory.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use std::time::Instant;

use super::cache::{intern_mesh, intern_mesh_profiled, lookup_source_mesh, remember_source_mesh};
use super::cache_key::source_mesh_key;
use super::profile::{GeometryProfile, hash_prim_path, record_mesh_sample};
use super::{PrimRoute, RouteCtx};
use crate::read::geom::{ReadMesh, ReadPointInstancer, read_mesh, read_point_instancer};
use openusd::schemas::geom::PointInstancer;
use openusd::sdf::Value;

/// Resolve schema `invisibleIds` into source-row indices.
fn invisible_indices(
    ctx: &RouteCtx,
    instance_count: usize,
) -> bevy::platform::collections::HashSet<usize> {
    let mut hidden_ids: bevy::platform::collections::HashSet<i64> =
        bevy::platform::collections::HashSet::default();
    if let Ok(Some(pi)) = PointInstancer::get(ctx.stage, ctx.path.clone()) {
        match pi.invisible_ids_attr().get::<Value>() {
            Ok(Some(Value::Int64Vec(v))) => hidden_ids.extend(v),
            Ok(Some(Value::IntVec(v))) => hidden_ids.extend(v.into_iter().map(i64::from)),
            _ => {}
        }
        let ids = match pi.ids_attr().get::<Value>() {
            Ok(Some(Value::Int64Vec(v))) => Some(v),
            Ok(Some(Value::IntVec(v))) => Some(v.into_iter().map(i64::from).collect()),
            _ => None,
        };
        if let Some(ids) = ids {
            return ids
                .into_iter()
                .enumerate()
                .filter_map(|(index, id)| hidden_ids.contains(&id).then_some(index))
                .collect();
        }
    }
    hidden_ids
        .into_iter()
        .filter_map(|id| usize::try_from(id).ok())
        .filter(|&index| index < instance_count)
        .collect()
}

/// A baked prototype's shared render handles.
type ProtoHandles = (Handle<Mesh>, Handle<StandardMaterial>);

/// Marker on entities spawned for a PointInstancer instance.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct UsdInstance;

/// Stable logical identity for a visible PointInstancer row.
///
/// The logical row is deliberately separate from Bevy's entity id and from
/// any future renderer instance index, so selection and live edits remain
/// stable if the rendering backend changes.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsdInstanceId {
    /// The source row in the PointInstancer positions/protoIndices arrays.
    pub index: u32,
    /// The source prototype relationship index for this row.
    pub prototype_index: u32,
}

/// Maps a `PointInstancer` prim to per-instance child entities.
pub struct PointInstancerRoute;

fn instance_transform(read: &ReadPointInstancer, i: usize) -> Transform {
    let t = read.positions[i];
    let mut xf = Transform::from_translation(Vec3::from_array(t));
    if let Some(o) = read.orientations.get(i) {
        // read_quat_array yields [w, x, y, z]; bevy is xyzw.
        xf.rotation = Quat::from_xyzw(o[1], o[2], o[3], o[0]);
    }
    if let Some(s) = read.scales.get(i) {
        xf.scale = Vec3::from_array(*s);
    }
    xf
}

impl PointInstancerRoute {
    /// Despawn instance children this route spawned on a previous project, so a
    /// reproject doesn't stack duplicate batches.
    fn clear_instances(world: &mut World, entity: Entity) {
        let existing: Vec<Entity> = world
            .get::<Children>(entity)
            .map(|c| c.iter().collect())
            .unwrap_or_default();
        for child in existing {
            if world.get::<UsdInstance>(child).is_some() {
                world.entity_mut(child).despawn();
            }
        }
    }
}

impl PrimRoute for PointInstancerRoute {
    fn matches(&self, ctx: &RouteCtx) -> bool {
        ctx.type_name.as_deref() == Some("PointInstancer")
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let Ok(Some(read)) = read_point_instancer(ctx.stage, ctx.path) else {
            return;
        };
        Self::clear_instances(world, entity);

        // Honor `invisibleIds` (read through the geom schema): those instances
        // are culled entirely.
        let invisible = invisible_indices(ctx, read.positions.len());

        let have_assets = world.get_resource::<Assets<Mesh>>().is_some()
            && world.get_resource::<Assets<StandardMaterial>>().is_some();

        // Bake each referenced prototype's mesh once; share the handles.
        let mut proto_cache: HashMap<usize, Option<ProtoHandles>> = HashMap::default();

        for i in 0..read.positions.len() {
            if invisible.contains(&i) {
                continue;
            }
            let xf = instance_transform(&read, i);
            let proto_idx = read
                .proto_indices
                .get(i)
                .and_then(|index| usize::try_from(*index).ok())
                .unwrap_or(0);

            let handles = if have_assets {
                proto_cache
                    .entry(proto_idx)
                    .or_insert_with(|| bake_prototype(ctx, world, &read, proto_idx))
                    .clone()
            } else {
                None
            };

            let mut e = world.spawn((
                UsdInstance,
                UsdInstanceId {
                    index: i as u32,
                    prototype_index: proto_idx as u32,
                },
                xf,
                Visibility::default(),
                ChildOf(entity),
            ));
            if let Some((mesh, material)) = handles {
                e.insert((Mesh3d(mesh), MeshMaterial3d(material)));
            }
        }
    }
}

/// Find the first renderable mesh in a prototype subtree.
///
/// USD PointInstancer relationships commonly target an Xform prototype whose
/// mesh is a child, rather than targeting a Mesh directly. The current Bevy
/// instance representation carries one mesh handle per logical instance, so a
/// prototype with multiple mesh children is intentionally left for a future
/// multi-part representation instead of silently drawing only one part.
fn read_prototype_mesh(
    stage: &openusd::usd::Stage,
    root: &openusd::sdf::Path,
) -> Option<(openusd::sdf::Path, ReadMesh)> {
    if let Ok(Some(mesh)) = read_mesh(stage, root) {
        return Some((root.clone(), mesh));
    }
    let children = stage.prim(root.clone()).children().ok()?;
    let mut result = None;
    for child in children {
        if let Some(found) = read_prototype_mesh(stage, child.path()) {
            if result.is_some() {
                return None;
            }
            result = Some(found);
        }
    }
    result
}

/// Bake the prototype at `proto_idx` into a shared `(Mesh, Material)`. Returns
/// `None` if the prototype path does not resolve to exactly one readable mesh.
fn bake_prototype(
    ctx: &RouteCtx,
    world: &mut World,
    read: &ReadPointInstancer,
    proto_idx: usize,
) -> Option<ProtoHandles> {
    let proto_root = read.prototypes.get(proto_idx)?;
    let (proto_path, mesh_read) = read_prototype_mesh(ctx.stage, proto_root)?;
    let profile_enabled = world
        .get_resource::<GeometryProfile>()
        .is_some_and(|profile| profile.enabled);
    let read_start = profile_enabled.then(Instant::now);
    let read_mesh_ms = read_start
        .map(|started| started.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or_default();
    let source_key = source_mesh_key(&mesh_read);
    let source_hit = lookup_source_mesh(world, source_key);
    let source_cache_lookup = world
        .get_resource::<super::cache::ProjectionCache>()
        .is_some();
    let source_cache_hit = source_hit.is_some();
    let (mesh_handle, build_metrics, intern_metrics) = if let Some(mesh_handle) = source_hit {
        (mesh_handle, Default::default(), Default::default())
    } else {
        let (mesh, build_metrics) = if profile_enabled {
            crate::mesh::mesh_from_usd_profiled(&mesh_read)
        } else {
            (crate::mesh::mesh_from_usd(&mesh_read), Default::default())
        };
        let (mesh_handle, intern_metrics) = if profile_enabled {
            intern_mesh_profiled(world, mesh)
        } else {
            (intern_mesh(world, mesh), Default::default())
        };
        if source_cache_lookup {
            remember_source_mesh(world, source_key, mesh_handle.clone());
        }
        (mesh_handle, build_metrics, intern_metrics)
    };
    if profile_enabled {
        let mut profile = world.resource_mut::<GeometryProfile>();
        record_mesh_sample(
            &mut profile,
            hash_prim_path(proto_path.as_str()),
            read_mesh_ms,
            build_metrics,
            intern_metrics,
            !source_cache_hit,
            source_cache_lookup,
            source_cache_hit,
        );
    }
    let material = super::fallback_material(world);
    Some((mesh_handle, material))
}
