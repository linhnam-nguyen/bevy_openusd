//! PointInstancer route (PLAN P4, in-repo): `UsdGeomPointInstancer` → one
//! child entity per instance, sharing the prototype's baked mesh.
//!
//! This is *point* instancing (an explicit position/orientation/scale table),
//! distinct from USD native scenegraph instancing.
//! Instance children reconcile by logical ID on reprojection. Occurrences of
//! one prototype share the prototype's mesh and material assets.

use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;

use super::cache::{intern_mesh, intern_mesh_profiled, lookup_source_mesh, remember_source_mesh};
use super::cache_key::source_mesh_key;
use super::instancer_dependency::PointInstancerDependencyIndex;
pub use super::instancer_state::{PointInstancerSelection, PointInstancerStats};
use super::profile::{GeometryProfile, hash_prim_path, record_mesh_sample};
use super::{PrimRoute, RouteCtx};
use crate::read::geom::{ReadMesh, ReadPointInstancer, read_point_instancer_at};

mod gpu_route;
mod prototype;
mod reconcile;
pub use prototype::UsdPrototypePart;

/// Resolve composed masks to source rows (upstream f2d474fe, route/instancer.rs).
fn invisible_indices(ctx: &RouteCtx, read: &ReadPointInstancer) -> anyhow::Result<HashSet<usize>> {
    use openusd::vt::Value;
    use openusd_schemas::geom::PointInstancerSchema;
    let mut hidden_ids = HashSet::<i64>::default();
    let prim = ctx.stage.prim(ctx.path.clone())?;
    match prim.metadata::<Value>("inactiveIds")? {
        Some(Value::Int64ListOp(op)) if op.is_explicit() => {
            hidden_ids.extend(op.explicit_items().iter().copied());
        }
        None => {}
        _ => anyhow::bail!("invalid composed inactiveIds metadata"),
    }
    if let Some(instancer) =
        openusd_schemas::geom::PointInstancer::get(ctx.stage, ctx.path.clone())?
    {
        let attr = instancer.invisible_ids_attr();
        let mask = ctx.time.map_or_else(
            || attr.get::<Value>(),
            |time| attr.get_at::<Value>(openusd::usd::TimeCode::new(time)),
        )?;
        match mask {
            Some(Value::Int64Vec(values)) => hidden_ids.extend(values),
            Some(Value::IntVec(values)) => hidden_ids.extend(values.into_iter().map(i64::from)),
            None => {}
            _ => anyhow::bail!("invalid invisibleIds array"),
        }
    }
    Ok((0..read.positions.len())
        .filter(|&row| hidden_ids.contains(&PointInstancerRoute::logical_id(read, row)))
        .collect())
}

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
    /// Authored logical ID, or the source row when `ids` is unauthored.
    pub logical_id: i64,
    /// The current source row in the PointInstancer arrays.
    pub source_index: u32,
    /// The source prototype relationship index for this row.
    pub prototype_index: u32,
}

/// Logical occurrence metadata shared by renderer and viewport adapters.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct UsdPointOccurrence {
    pub instancer: std::sync::Arc<str>,
    pub logical_id: i64,
    pub authored: bool,
    /// Zero for initial load, otherwise the projection that admitted this row.
    pub incarnation: u64,
    pub masked: bool,
}

/// Bounding volume of visible prototype parts in occurrence-local space.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct UsdOccurrenceBounds {
    pub min: Vec3,
    pub max: Vec3,
}

/// Maps a `PointInstancer` prim to per-instance child entities.
pub struct PointInstancerRoute;

fn instance_transform(read: &ReadPointInstancer, i: usize) -> Transform {
    let t = read.positions[i];
    let mut xf = Transform::from_translation(Vec3::from_array(t));
    if let Some(o) = read.orientations.get(i) {
        // read_quat_array yields [w, x, y, z]; bevy is xyzw.
        xf.rotation = Quat::from_xyzw(o[1], o[2], o[3], o[0]).normalize();
    }
    if let Some(s) = read.scales.get(i) {
        xf.scale = Vec3::from_array(*s);
    }
    xf
}

impl PointInstancerRoute {
    fn record_stats(world: &mut World, update: impl FnOnce(&mut PointInstancerStats)) {
        if let Some(mut stats) = world.get_resource_mut::<PointInstancerStats>() {
            update(&mut stats);
        }
    }

    fn logical_id(read: &ReadPointInstancer, source_index: usize) -> i64 {
        read.ids
            .as_ref()
            .and_then(|ids| ids.get(source_index).copied())
            .unwrap_or(source_index as i64)
    }

    fn patch_transforms(
        read: &ReadPointInstancer,
        invisible: &HashSet<usize>,
        world: &mut World,
        entity: Entity,
    ) -> Option<usize> {
        let gpu = crate::route::gpu_instancing::has_draws(world, entity);
        let children: Vec<Entity> = world
            .get::<Children>(entity)
            .map(|children| children.iter().collect())
            .unwrap_or_default();
        let mut by_logical_id = HashMap::with_capacity(children.len());
        for child in children {
            let Some(instance_id) = world.get::<UsdInstanceId>(child).copied() else {
                continue;
            };
            if by_logical_id
                .insert(instance_id.logical_id, (child, instance_id.prototype_index))
                .is_some()
            {
                return None;
            }
        }

        let visible_count = read
            .positions
            .iter()
            .enumerate()
            .filter(|(index, _)| !invisible.contains(index))
            .count();
        if read.positions.len() != by_logical_id.len() {
            return None;
        }

        let mut updates = Vec::with_capacity(read.positions.len());
        for source_index in 0..read.positions.len() {
            let logical_id = Self::logical_id(read, source_index);
            let (child, prototype_index) = by_logical_id.get(&logical_id).copied()?;
            if prototype_index != read.proto_indices[source_index] as u32
                || world.get::<UsdInstanceId>(child)?.source_index != source_index as u32
                || (world.get::<Visibility>(child) == Some(&Visibility::Hidden))
                    != invisible.contains(&source_index)
            {
                return None;
            }
            let transform = instance_transform(read, source_index);
            if gpu && !invisible.contains(&source_index) && transform.scale.min_element() <= 1e-6 {
                return None;
            }
            if world.get::<Transform>(child) != Some(&transform) {
                updates.push((child, logical_id, transform));
            }
        }

        if updates.is_empty() {
            return Some(visible_count);
        }
        if gpu {
            let changed = updates
                .iter()
                .map(|(_, id, transform)| (*id, *transform))
                .collect();
            crate::route::gpu_instancing::patch_transforms(world, entity, &changed);
        }
        for (child, _, transform) in updates {
            world.entity_mut(child).insert(transform);
        }
        reconcile::sync_instancer_extent(world, entity);
        Some(visible_count)
    }
}

impl PrimRoute for PointInstancerRoute {
    fn matches(&self, ctx: &RouteCtx) -> bool {
        ctx.type_name.as_deref() == Some("PointInstancer")
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let read_validated = || {
            let Some(read) = read_point_instancer_at(ctx.stage, ctx.path, ctx.time)? else {
                anyhow::bail!("positions or protoIndices are unavailable");
            };
            reconcile::validate(&read).map_err(anyhow::Error::msg)?;
            let invisible = invisible_indices(ctx, &read)?;
            Ok::<_, anyhow::Error>((read, invisible))
        };
        let (read, invisible) = match read_validated() {
            Ok(data) => data,
            Err(reason) => {
                bevy::log::warn!(target: "usd_bevy::route::instancer", "{}: {reason}", ctx.prim_str());
                return;
            }
        };
        let mut prototype_roots = read
            .prototypes
            .iter()
            .map(|prototype| prototype.as_str().to_string())
            .collect::<Vec<_>>();
        let totals = reconcile::apply(ctx, world, entity, &read, &invisible);
        prototype_roots.extend(totals.prototype_dependencies);
        world.resource_scope(
            |world, mut dependencies: Mut<PointInstancerDependencyIndex>| {
                let mut paths = world.resource_mut::<crate::live::PathStore>();
                dependencies.replace_instancer(&mut paths, ctx.prim_str(), prototype_roots);
            },
        );
        Self::record_stats(world, |stats| {
            stats.full_projects += 1;
            stats.instance_spawns += totals.spawned;
            stats.instance_despawns += totals.despawned;
            stats.transform_updates += totals.reused;
        });
    }

    fn patch(&self, ctx: &RouteCtx, world: &mut World, entity: Entity, changed: &[&str]) {
        let transform_only = !changed.is_empty()
            && changed
                .iter()
                .all(|property| matches!(*property, "positions" | "orientations" | "scales"));
        if transform_only
            && let Ok(Some(read)) = read_point_instancer_at(ctx.stage, ctx.path, ctx.time)
            && reconcile::validate(&read).is_ok()
            && let Ok(invisible) = invisible_indices(ctx, &read)
        {
            if let Some(updated) = Self::patch_transforms(&read, &invisible, world, entity) {
                Self::record_stats(world, |stats| {
                    stats.sparse_transform_patches += 1;
                    stats.transform_updates += updated as u64;
                });
                return;
            }
        }
        self.project(ctx, world, entity);
    }
}

fn bake_mesh(
    world: &mut World,
    proto_path: &openusd::sdf::Path,
    mesh_read: ReadMesh,
    read_mesh_ms: f64,
) -> Handle<Mesh> {
    let profile_enabled = world
        .get_resource::<GeometryProfile>()
        .is_some_and(|profile| profile.enabled);
    let source_key = world
        .get_resource::<super::cache::ProjectionCache>()
        .and_then(|_| source_mesh_key(&mesh_read));
    let source_hit = source_key
        .as_ref()
        .and_then(|key| lookup_source_mesh(world, key));
    let source_cache_lookup = source_key.is_some();
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
        if let Some(source_key) = source_key {
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
    mesh_handle
}
