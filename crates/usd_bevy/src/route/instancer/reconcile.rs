//! Keyed CPU instance reconciliation. This is the entity fallback and the
//! reference identity authority for a future GPU representation.

use super::{
    UsdInstance, UsdInstanceId, UsdOccurrenceBounds, UsdPointOccurrence, gpu_route,
    instance_transform, prototype,
};
use crate::read::geom::ReadPointInstancer;
use crate::route::RouteCtx;
use crate::route::gpu_instancing;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use std::sync::Arc;

#[derive(Component, PartialEq, Eq)]
struct IdentityTopology {
    authored: bool,
    anonymous_paths: Vec<openusd::sdf::Path>,
    anonymous_indices: Vec<i32>,
}

#[derive(Component)]
struct OccurrenceSequence {
    instancer: Arc<str>,
    revision: u64,
}

#[derive(Default)]
pub(super) struct Totals {
    pub spawned: u64,
    pub despawned: u64,
    pub reused: u64,
    pub prototype_dependencies: Vec<String>,
}

pub(super) fn validate(read: &ReadPointInstancer) -> Result<(), &'static str> {
    let count = read.positions.len();
    if count > u32::MAX as usize {
        return Err("source row count overflows u32");
    }
    if read.proto_indices.len() != count {
        return Err("protoIndices length does not match positions");
    }
    if !read.orientations.is_empty() && read.orientations.len() != count {
        return Err("orientations length does not match positions");
    }
    if !read.scales.is_empty() && read.scales.len() != count {
        return Err("scales length does not match positions");
    }
    if let Some(ids) = &read.ids {
        if ids.len() != count {
            return Err("ids length does not match positions");
        }
        if !ids.windows(2).all(|pair| pair[0] < pair[1]) {
            let mut unique = HashSet::with_capacity(ids.len());
            if ids.iter().any(|id| !unique.insert(*id)) {
                return Err("duplicate authored instance ID");
            }
        }
    }
    if read.positions.iter().flatten().any(|v| !v.is_finite()) {
        return Err("non-finite position");
    }
    if read.scales.iter().flatten().any(|v| !v.is_finite()) {
        return Err("non-finite scale");
    }
    if read.orientations.iter().any(|value| {
        let norm_squared = value.iter().map(|v| v * v).sum::<f32>();
        !norm_squared.is_finite() || norm_squared <= f32::MIN_POSITIVE
    }) {
        return Err("orientations must be finite nonzero quaternions");
    }
    if read
        .proto_indices
        .iter()
        .any(|i| *i < 0 || usize::try_from(*i).map_or(true, |i| i >= read.prototypes.len()))
    {
        return Err("out-of-range prototype index");
    }
    Ok(())
}

/// Reconcile direct logical children by authored instance ID. Anonymous
/// instances retain row-scoped identity; changing topology without ids cannot
/// promise stable identity across a reorder.
pub(super) fn apply(
    ctx: &RouteCtx,
    world: &mut World,
    parent: Entity,
    read: &ReadPointInstancer,
    invisible: &HashSet<usize>,
) -> Totals {
    let have_assets = world.contains_resource::<Assets<Mesh>>()
        && world.contains_resource::<Assets<StandardMaterial>>();
    let mut proto_cache = HashMap::<usize, prototype::Prototype>::default();
    let children: Vec<Entity> = world
        .get::<Children>(parent)
        .map(|children| children.iter().collect())
        .unwrap_or_default();
    let mut current: HashMap<i64, Entity> = HashMap::with_capacity(children.len());
    let mut stale = Vec::new();
    let mut order = Vec::with_capacity(children.len().max(read.positions.len()));
    let authored = read.ids.is_some();
    let topology = IdentityTopology {
        authored,
        anonymous_paths: if authored {
            Vec::new()
        } else {
            read.prototypes.clone()
        },
        anonymous_indices: if authored {
            Vec::new()
        } else {
            read.proto_indices.clone()
        },
    };
    let topology_changed = world.get::<IdentityTopology>(parent) != Some(&topology);
    if topology_changed {
        world.entity_mut(parent).insert(topology);
    }
    if world.get::<OccurrenceSequence>(parent).is_none() {
        world.entity_mut(parent).insert(OccurrenceSequence {
            instancer: Arc::from(ctx.prim_str()),
            revision: 0,
        });
    } else {
        let mut sequence = world.get_mut::<OccurrenceSequence>(parent).unwrap();
        sequence.revision = sequence
            .revision
            .checked_add(1)
            .expect("occurrence sequence exhausted");
    }
    let sequence = world.get::<OccurrenceSequence>(parent).unwrap();
    let (instancer, incarnation) = (sequence.instancer.clone(), sequence.revision);
    for child in children {
        if world
            .get::<gpu_instancing::UsdInstancedDraw>(child)
            .is_some()
        {
            continue;
        }
        if world.get::<UsdInstance>(child).is_none() {
            order.push(child);
            continue;
        }
        match world.get::<UsdInstanceId>(child).map(|id| id.logical_id) {
            Some(key) if !topology_changed && !current.contains_key(&key) => {
                current.insert(key, child);
            }
            _ => stale.push(child),
        }
    }
    let selection = world
        .get_resource::<gpu_instancing::PointPresentations>()
        .and_then(|presentations| {
            Some((
                presentations.over_budget,
                presentations.selected.get(ctx.prim_str())?,
            ))
        });
    let mut promoted = HashSet::<i64>::default();
    let mut force_entity = false;
    if let Some((over_budget, selected)) = selection {
        for (id, incarnation) in selected {
            if current
                .get(id)
                .and_then(|entity| world.get::<UsdPointOccurrence>(*entity))
                .is_some_and(|occurrence| {
                    occurrence.incarnation == *incarnation && !occurrence.masked
                })
            {
                promoted.insert(*id);
            }
        }
        force_entity = over_budget && !promoted.is_empty();
    }
    let draw_plan = (have_assets && !force_entity)
        .then(|| gpu_instancing::chunk_capacity(world, read.positions.len()))
        .flatten()
        .and_then(|capacity| {
            for &index in &read.proto_indices {
                let index = index as usize;
                proto_cache
                    .entry(index)
                    .or_insert_with(|| prototype::prepare(ctx, world, &read.prototypes[index]));
            }
            gpu_route::DrawPlan::prepare(
                world,
                parent,
                read,
                invisible,
                &promoted,
                &proto_cache,
                capacity,
            )
        });
    if !promoted.is_empty() {
        bevy::log::info!(target: "usd_bevy::route::gpu_instancing",
            instancer = ctx.prim_str(), count = promoted.len(), force_entity,
            "reconciled selected point-instancer presentations");
    }
    let mut totals = Totals::default();
    for row in 0..read.positions.len() {
        let masked = invisible.contains(&row);
        let logical_id = read.ids.as_ref().map_or(row as i64, |ids| ids[row]);
        let proto_idx = read.proto_indices[row] as usize;
        let id = UsdInstanceId {
            logical_id,
            source_index: row as u32,
            prototype_index: proto_idx as u32,
        };
        let xf = instance_transform(read, row);
        let existing = current.remove(&logical_id);
        let child = if let Some(entity) = existing {
            entity
        } else {
            totals.spawned += 1;
            world
                .spawn((
                    UsdInstance,
                    id,
                    UsdPointOccurrence {
                        instancer: instancer.clone(),
                        logical_id,
                        authored,
                        incarnation,
                        masked,
                    },
                    xf,
                    Visibility::default(),
                    ChildOf(parent),
                ))
                .id()
        };
        order.push(child);
        if existing.is_some() {
            let mut e = world.entity_mut(child);
            if e.get::<UsdInstanceId>() != Some(&id) {
                e.insert(id);
            }
            if e.get::<Transform>() != Some(&xf) {
                e.insert(xf);
                totals.reused += 1;
            }
            if let Some(current) = e.get::<UsdPointOccurrence>()
                && current.masked != masked
            {
                e.insert(UsdPointOccurrence {
                    masked,
                    ..current.clone()
                });
            }
        }
        let mut e = world.entity_mut(child);
        let visibility = if masked {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if e.get::<Visibility>() != Some(&visibility) {
            e.insert(visibility);
        }
        let prepared = have_assets.then(|| {
            &*proto_cache
                .entry(proto_idx)
                .or_insert_with(|| prototype::prepare(ctx, world, &read.prototypes[proto_idx]))
        });
        let bounds = prepared
            .and_then(|prototype| prototype.bounds)
            .filter(|_| !masked);
        if world.get::<UsdOccurrenceBounds>(child) != bounds.as_ref() {
            let mut child = world.entity_mut(child);
            if let Some(bounds) = bounds {
                child.insert(bounds);
            } else {
                child.remove::<UsdOccurrenceBounds>();
            }
        }
        prototype::reconcile_parts(
            world,
            child,
            prepared.filter(|_| draw_plan.is_none() || promoted.contains(&logical_id)),
            masked,
        );
    }
    if let Some(mut selection) = world.get_resource_mut::<super::PointInstancerSelection>()
        && selection.instancer_path.as_deref() == Some(ctx.prim_str())
        && let Some(id) = selection.logical_id
        && (topology_changed || current.contains_key(&id))
    {
        selection.clear();
    }
    stale.extend(current.into_values());
    for entity in stale {
        if world.get_entity(entity).is_ok() {
            prototype::reconcile_parts(world, entity, None, true);
            world.entity_mut(entity).despawn();
            totals.despawned += 1;
        }
    }
    if world.get::<Children>(parent).is_some_and(|children| {
        !children
            .iter()
            .filter(|child| {
                world
                    .get::<gpu_instancing::UsdInstancedDraw>(*child)
                    .is_none()
            })
            .eq(order.iter().copied())
    }) {
        let ranks: HashMap<_, _> = order
            .into_iter()
            .enumerate()
            .map(|(rank, entity)| (entity, rank))
            .collect();
        if let Some(mut children) = world.get_mut::<Children>(parent) {
            children.sort_by_key(|entity| ranks.get(entity).copied().unwrap_or(usize::MAX));
        }
    }
    if let Some(plan) = draw_plan {
        plan.publish(world, parent, instancer);
    } else {
        gpu_instancing::clear(world, parent);
    }
    if let Some(picker) = world.get_resource::<gpu_instancing::GpuPointPicker>() {
        picker.invalidate_representation(
            world.contains_resource::<gpu_instancing::SelectionOnlyPointProjection>(),
        );
    }
    sync_instancer_extent(world, parent);
    totals.prototype_dependencies = proto_cache
        .into_values()
        .flat_map(|prototype| prototype.dependencies)
        .collect();
    totals
}

pub(super) fn sync_instancer_extent(world: &mut World, parent: Entity) {
    use crate::route::geom::UsdLocalExtent;

    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    if let Some(children) = world.get::<Children>(parent) {
        for child in children.iter() {
            let (Some(bounds), Some(transform)) = (
                world.get::<UsdOccurrenceBounds>(child),
                world.get::<Transform>(child),
            ) else {
                continue;
            };
            let affine = transform.compute_affine();
            let center = affine.transform_point3(bounds.min * 0.5 + bounds.max * 0.5);
            let half = bounds.max * 0.5 - bounds.min * 0.5;
            let matrix = affine.matrix3;
            let radius = Vec3::from(
                matrix.x_axis.abs() * half.x
                    + matrix.y_axis.abs() * half.y
                    + matrix.z_axis.abs() * half.z,
            );
            if center.is_finite() && radius.is_finite() {
                min = min.min(center - radius);
                max = max.max(center + radius);
            }
        }
    }
    let extent = (min.is_finite() && max.is_finite()).then(|| UsdLocalExtent {
        min: min.to_array(),
        max: max.to_array(),
    });
    if world.get::<UsdLocalExtent>(parent) != extent.as_ref() {
        let mut parent = world.entity_mut(parent);
        if let Some(extent) = extent {
            parent.insert(extent);
        } else {
            parent.remove::<UsdLocalExtent>();
        }
    }
}
