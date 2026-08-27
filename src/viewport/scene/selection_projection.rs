//! Cached renderer projection of logical selection anchors.
//!
//! Scene anchors are resolved to projected renderables once per selection or
//! projection change. Presentation systems consume this shared set instead of
//! walking every selected hierarchy independently.

use std::collections::{HashMap, HashSet};

use bevy::camera::primitives::Aabb;
use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use usd_bevy::UsdLocalExtent;
use viewport_protocol::SceneAnchor;

use crate::viewport::api::SceneAnchorIndex;
use crate::viewport::scene::SelectedTargets;

#[path = "selection_projection_bounds.rs"]
mod bounds;

use bounds::{
    aggregate_bounds, bounds_for_entities, collect_mesh_descendants, replace_target_bounds,
};

pub(crate) use bounds::ProjectedWorldBounds;

#[derive(Resource, Debug, Default)]
pub(crate) struct SelectedRenderableProjection {
    target_renderables: HashMap<SceneAnchor, HashSet<Entity>>,
    target_bounds: HashMap<SceneAnchor, ProjectedWorldBounds>,
    renderables: HashSet<Entity>,
    renderable_refcounts: HashMap<Entity, usize>,
    added_renderables: HashSet<Entity>,
    removed_renderables: HashSet<Entity>,
    aggregate_bounds: Option<ProjectedWorldBounds>,
    last_selection_revision: Option<u64>,
    last_scene_revision: Option<u64>,
    generation: u64,
    bounds_generation: u64,
    resolution_count: u64,
}

impl SelectedRenderableProjection {
    pub(crate) fn renderables(&self) -> &HashSet<Entity> {
        &self.renderables
    }

    pub(crate) fn added_renderables(&self) -> &HashSet<Entity> {
        &self.added_renderables
    }

    pub(crate) fn removed_renderables(&self) -> &HashSet<Entity> {
        &self.removed_renderables
    }

    pub(crate) fn aggregate_bounds(&self) -> Option<ProjectedWorldBounds> {
        self.aggregate_bounds
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn bounds_generation(&self) -> u64 {
        self.bounds_generation
    }

    pub(crate) fn resolution_count(&self) -> u64 {
        self.resolution_count
    }
}

#[allow(clippy::type_complexity)]
pub(in crate::viewport) fn sync_selected_renderable_projection(
    mut selection: ResMut<SelectedTargets>,
    scene_index: Res<SceneAnchorIndex>,
    mut projection: ResMut<SelectedRenderableProjection>,
    hierarchy: Query<(Option<&Children>, Option<&Mesh3d>)>,
    geometry: Query<(
        Option<&GlobalTransform>,
        Option<&Mesh3d>,
        Option<&Aabb>,
        Option<&UsdLocalExtent>,
    )>,
    topology_changed: Query<
        Entity,
        Or<(
            Added<Mesh3d>,
            Changed<Mesh3d>,
            Added<Children>,
            Changed<Children>,
        )>,
    >,
    geometry_changed: Query<
        Entity,
        Or<(
            Added<GlobalTransform>,
            Changed<GlobalTransform>,
            Added<Aabb>,
            Changed<Aabb>,
            Added<UsdLocalExtent>,
            Changed<UsdLocalExtent>,
        )>,
    >,
    mut removed_meshes: RemovedComponents<Mesh3d>,
    mut removed_children: RemovedComponents<Children>,
) {
    let targets = &selection.0.targets;
    let scene_revision = scene_index.revision();
    let scene_changed = projection.last_scene_revision != Some(scene_revision);
    let selection_changed = projection.last_selection_revision != Some(selection.revision());
    let topology_changed = !topology_changed.is_empty()
        || removed_meshes.read().next().is_some()
        || removed_children.read().next().is_some();
    let geometry_changed = geometry_changed.iter().collect::<HashSet<_>>();
    projection.added_renderables.clear();
    projection.removed_renderables.clear();
    if !scene_changed && !selection_changed && !topology_changed && geometry_changed.is_empty() {
        return;
    }

    let full_rebuild =
        projection.last_selection_revision.is_none() || scene_changed || topology_changed;
    let mut mapping_changed = full_rebuild;
    let mut bounds_changed = full_rebuild;
    let mut aggregate_can_extend = !full_rebuild;
    let mut added_bounds = Vec::new();

    if full_rebuild {
        let previous_renderables = std::mem::take(&mut projection.renderables);
        projection.target_renderables.clear();
        projection.target_bounds.clear();
        projection.renderable_refcounts.clear();
        for target in targets {
            insert_target_projection(target, &scene_index, &hierarchy, &geometry, &mut projection);
        }
        projection.added_renderables = projection
            .renderables
            .difference(&previous_renderables)
            .copied()
            .collect();
        projection.removed_renderables = previous_renderables
            .difference(&projection.renderables)
            .copied()
            .collect();
        selection.clear_pending_delta();
    } else {
        let pending_delta = selection.pending_delta().clone();
        for target in pending_delta.removed {
            if let Some(renderables) = projection.target_renderables.remove(&target) {
                remove_target_renderables(&mut projection, &renderables);
                mapping_changed = true;
            }
            if projection.target_bounds.remove(&target).is_some() {
                bounds_changed = true;
                aggregate_can_extend = false;
            }
        }

        for target in pending_delta.added {
            let previous_bounds = projection.target_bounds.get(&target).copied();
            insert_target_projection(
                &target,
                &scene_index,
                &hierarchy,
                &geometry,
                &mut projection,
            );
            if let Some(bounds) = projection.target_bounds.get(&target).copied()
                && previous_bounds.is_none()
            {
                added_bounds.push(bounds);
            }
            mapping_changed = true;
            bounds_changed = true;
        }

        if !geometry_changed.is_empty() {
            for target in targets {
                let Some(renderables) = projection.target_renderables.get(target) else {
                    continue;
                };
                if renderables
                    .iter()
                    .any(|entity| geometry_changed.contains(entity))
                {
                    let previous = projection.target_bounds.get(target).copied();
                    let next = bounds_for_entities(renderables, &geometry);
                    if next != previous {
                        bounds_changed = true;
                        aggregate_can_extend = false;
                        replace_target_bounds(&mut projection, target, next);
                    }
                }
            }
        }
        selection.clear_pending_delta();
    }

    if mapping_changed {
        projection.generation = projection.generation.saturating_add(1);
    }
    if bounds_changed {
        if aggregate_can_extend {
            for bounds in added_bounds {
                if let Some(current) = &mut projection.aggregate_bounds {
                    current.include(bounds);
                } else {
                    projection.aggregate_bounds = Some(bounds);
                }
            }
        } else {
            projection.aggregate_bounds = aggregate_bounds(&projection.target_bounds);
        }
        projection.bounds_generation = projection.bounds_generation.saturating_add(1);
    }
    projection.last_selection_revision = Some(selection.revision());
    projection.last_scene_revision = Some(scene_revision);
}

fn insert_target_projection(
    target: &SceneAnchor,
    scene_index: &SceneAnchorIndex,
    hierarchy: &Query<(Option<&Children>, Option<&Mesh3d>)>,
    geometry: &Query<(
        Option<&GlobalTransform>,
        Option<&Mesh3d>,
        Option<&Aabb>,
        Option<&UsdLocalExtent>,
    )>,
    projection: &mut SelectedRenderableProjection,
) {
    projection.resolution_count = projection.resolution_count.saturating_add(1);
    let Some(root) = scene_index.resolve(target) else {
        projection
            .target_renderables
            .insert(target.clone(), HashSet::new());
        return;
    };
    let renderables = collect_mesh_descendants(root, hierarchy);
    let bounds = bounds_for_entities(&renderables, geometry);
    add_target_renderables(projection, &renderables);
    projection
        .target_renderables
        .insert(target.clone(), renderables);
    replace_target_bounds(projection, target, bounds);
}

fn add_target_renderables(
    projection: &mut SelectedRenderableProjection,
    renderables: &HashSet<Entity>,
) {
    for entity in renderables {
        let count = projection.renderable_refcounts.entry(*entity).or_default();
        let was_unselected = *count == 0;
        *count += 1;
        projection.renderables.insert(*entity);
        if was_unselected && !projection.removed_renderables.remove(entity) {
            projection.added_renderables.insert(*entity);
        }
    }
}

fn remove_target_renderables(
    projection: &mut SelectedRenderableProjection,
    renderables: &HashSet<Entity>,
) {
    for entity in renderables {
        let Some(count) = projection.renderable_refcounts.get_mut(entity) else {
            continue;
        };
        *count -= 1;
        if *count == 0 {
            projection.renderable_refcounts.remove(entity);
            projection.renderables.remove(entity);
            if !projection.added_renderables.remove(entity) {
                projection.removed_renderables.insert(*entity);
            }
        }
    }
}
