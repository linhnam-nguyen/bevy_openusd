//! Admission and packed drawing inputs for supported PointInstancers.

use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use std::sync::Arc;

use crate::read::geom::ReadPointInstancer;
use crate::route::gpu_instancing::{
    self, GpuInstance, MAX_TRIANGLE_WORK, PackedRow, PreparedGpuMesh,
};

use super::prototype::Prototype;
use super::{PointInstancerRoute, instance_transform};

struct DrawGroup {
    prototype: Vec<PreparedGpuMesh>,
    rows: Vec<PackedRow>,
    triangles: usize,
}

pub(super) struct DrawPlan {
    groups: Vec<(usize, DrawGroup)>,
    capacity: usize,
}

impl DrawPlan {
    pub(super) fn prepare(
        world: &World,
        entity: Entity,
        read: &ReadPointInstancer,
        invisible: &HashSet<usize>,
        promoted: &HashSet<i64>,
        prototypes: &HashMap<usize, Prototype>,
        capacity: usize,
    ) -> Option<Self> {
        let local = world.get::<Transform>(entity)?;
        if local.scale.min_element() <= 1e-6 {
            return None;
        }
        let mut groups = HashMap::<usize, DrawGroup>::default();
        let mut work = 0usize;
        for row in 0..read.positions.len() {
            let logical_id = PointInstancerRoute::logical_id(read, row);
            let visible = !invisible.contains(&row);
            let enabled = visible && !promoted.contains(&logical_id);
            let transform = instance_transform(read, row);
            // Reflected and singular instance transforms use the established
            // entity rasterizer's winding/culling behavior.
            if visible && transform.scale.min_element() <= 1e-6 {
                return None;
            }
            let index = read.proto_indices[row] as usize;
            let group = match groups.entry(index) {
                bevy::platform::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                bevy::platform::collections::hash_map::Entry::Vacant(entry) => {
                    let prototype = prototypes.get(&index)?.gpu_meshes(world)?;
                    let triangles = prototype.iter().map(PreparedGpuMesh::triangles).sum();
                    entry.insert(DrawGroup {
                        prototype,
                        rows: Vec::new(),
                        triangles,
                    })
                }
            };
            work = work.checked_add(group.triangles)?;
            if work > MAX_TRIANGLE_WORK {
                return None;
            }
            if group.prototype.is_empty() {
                continue;
            }
            let packed =
                GpuInstance::new(transform.translation, transform.rotation, transform.scale);
            group.rows.push(PackedRow {
                logical_id,
                transform: packed,
                enabled: u32::from(enabled),
            });
        }
        let mut sorted: Vec<_> = groups
            .into_iter()
            .filter(|(_, group)| !group.rows.is_empty())
            .collect();
        sorted.sort_unstable_by_key(|(index, _)| *index);
        for (_, group) in &mut sorted {
            group.rows.sort_unstable_by_key(|row| row.logical_id);
        }
        (!sorted.is_empty()).then_some(Self {
            groups: sorted,
            capacity,
        })
    }

    pub(super) fn publish(self, world: &mut World, entity: Entity, instancer: Arc<str>) {
        let mut chunks = 0;
        let mut draws = 0;
        let mut rows = 0;
        let mut enabled_rows = 0;
        let mut previous = gpu_instancing::existing_draws(world, entity);
        for (prototype_index, group) in self.groups {
            let count = group.rows.len().div_ceil(self.capacity);
            chunks += count;
            draws += count * group.prototype.len();
            rows += group.rows.len();
            enabled_rows += group.rows.iter().filter(|row| row.enabled != 0).count();
            gpu_instancing::reconcile_draws(
                world,
                entity,
                instancer.clone(),
                prototype_index,
                &group.prototype,
                &group.rows,
                self.capacity,
                &mut previous,
            );
        }
        for entity in previous.into_values() {
            world.despawn(entity);
        }
        bevy::log::info!(
            target: "usd_bevy::route::gpu_instancing",
            rows, enabled_rows, chunks, draws,
            row_buffer_bytes = rows * std::mem::size_of::<GpuInstance>(),
            "published packed point-instancer draws"
        );
    }
}
