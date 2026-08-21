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

use super::cache::MeshInternMetrics;
use super::profile::{GeometryProfile, hash_prim_path, record_mesh_sample};
use super::{PrimRoute, RouteCtx};
use crate::read::geom::{ReadPointInstancer, read_point_instancer};
use openusd::schemas::geom::PointInstancer;
use openusd::sdf::Value;

/// Instance indices marked invisible via the schema's `invisibleIds`.
fn invisible_ids(ctx: &RouteCtx) -> bevy::platform::collections::HashSet<i64> {
    let mut set = bevy::platform::collections::HashSet::default();
    if let Ok(Some(pi)) = PointInstancer::get(ctx.stage, ctx.path.clone()) {
        match pi.invisible_ids_attr().get::<Value>() {
            Ok(Some(Value::Int64Vec(v))) => set.extend(v),
            Ok(Some(Value::IntVec(v))) => set.extend(v.into_iter().map(i64::from)),
            _ => {}
        }
    }
    set
}

/// A baked prototype's shared render handles.
type ProtoHandles = (Handle<Mesh>, Handle<StandardMaterial>);

/// Marker on entities spawned for a PointInstancer instance.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct UsdInstance;

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
        let invisible = invisible_ids(ctx);

        let have_assets = world.get_resource::<Assets<Mesh>>().is_some()
            && world.get_resource::<Assets<StandardMaterial>>().is_some();

        // Bake each referenced prototype's mesh once; share the handles.
        let mut proto_cache: HashMap<usize, Option<ProtoHandles>> = HashMap::default();

        for i in 0..read.positions.len() {
            if invisible.contains(&(i as i64)) {
                continue;
            }
            let xf = instance_transform(&read, i);
            let proto_idx = read.proto_indices.get(i).copied().unwrap_or(0) as usize;

            let handles = if have_assets {
                proto_cache
                    .entry(proto_idx)
                    .or_insert_with(|| bake_prototype(ctx, world, &read, proto_idx))
                    .clone()
            } else {
                None
            };

            let mut e = world.spawn((UsdInstance, xf, Visibility::default(), ChildOf(entity)));
            if let Some((mesh, material)) = handles {
                e.insert((Mesh3d(mesh), MeshMaterial3d(material)));
            }
        }
    }
}

/// Bake the prototype at `proto_idx` into a shared `(Mesh, Material)`. Returns
/// `None` if the prototype path doesn't resolve to a readable mesh.
fn bake_prototype(
    ctx: &RouteCtx,
    world: &mut World,
    read: &ReadPointInstancer,
    proto_idx: usize,
) -> Option<ProtoHandles> {
    let proto_path = read.prototypes.get(proto_idx)?;
    let profile_enabled = world
        .get_resource::<GeometryProfile>()
        .is_some_and(|profile| profile.enabled);
    let read_start = profile_enabled.then(Instant::now);
    let mesh_read = crate::read::geom::read_mesh(ctx.stage, proto_path)
        .ok()
        .flatten()?;
    let read_mesh_ms = read_start
        .map(|started| started.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or_default();
    let (mesh, build_metrics) = if profile_enabled {
        crate::mesh::mesh_from_usd_profiled(&mesh_read)
    } else {
        (crate::mesh::mesh_from_usd(&mesh_read), Default::default())
    };
    let allocation_start = profile_enabled.then(Instant::now);
    let mesh_handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
    let allocation_ms = allocation_start
        .map(|started| started.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or_default();
    if profile_enabled {
        let mut profile = world.resource_mut::<GeometryProfile>();
        record_mesh_sample(
            &mut profile,
            hash_prim_path(proto_path.as_str()),
            read_mesh_ms,
            build_metrics,
            MeshInternMetrics {
                total_ms: allocation_ms,
                allocation_ms,
                ..Default::default()
            },
        );
    }
    let material = world
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    Some((mesh_handle, material))
}
