//! Cached renderer projection of logical selection anchors.
//!
//! Scene anchors are resolved to projected renderables once per selection or
//! projection change. Presentation systems consume this shared set instead of
//! walking every selected hierarchy independently.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::camera::primitives::Aabb;
use bevy::ecs::hierarchy::{ChildOf, Children};
use bevy::ecs::lifecycle::{Insert, Remove};
use bevy::ecs::observer::On;
use bevy::prelude::*;
use usd_bevy::UsdLocalExtent;
use viewport_protocol::SceneAnchor;

use crate::viewport::api::{SceneAnchorIndex, ViewerSettingsState};
use crate::viewport::scene::{SelectedPrim, SelectedTargets, SelectionPresentationPolicy};

#[path = "selection_projection_bounds.rs"]
mod bounds;
#[path = "selection_projection_membership.rs"]
mod membership;
#[path = "selection_projection_topology.rs"]
mod topology;

use bounds::{
    PendingTargetBounds, advance_pending_bounds, aggregate_bounds, schedule_target_bounds,
};
use membership::{
    advance_pending_removals, advance_pending_targets, reconcile_targets, schedule_targets,
};
use topology::advance_pending_topology;

pub(crate) use bounds::ProjectedWorldBounds;

#[derive(Resource, Debug, Default)]
pub(crate) struct SelectedRenderableProjection {
    target_renderables: HashMap<SceneAnchor, HashSet<Entity>>,
    /// Stable discovery order lets bounds work advance without re-walking a
    /// target's membership set after every geometry or topology delta.
    target_renderable_order: HashMap<SceneAnchor, Vec<Entity>>,
    target_roots: HashMap<SceneAnchor, Entity>,
    roots_by_entity: HashMap<Entity, HashSet<SceneAnchor>>,
    renderable_targets: HashMap<Entity, HashSet<SceneAnchor>>,
    target_bounds: HashMap<SceneAnchor, ProjectedWorldBounds>,
    renderables: HashSet<Entity>,
    renderable_refcounts: HashMap<Entity, usize>,
    added_renderables: HashSet<Entity>,
    removed_renderables: HashSet<Entity>,
    aggregate_bounds: Option<ProjectedWorldBounds>,
    last_selection_revision: Option<u64>,
    last_scene_revision: Option<u64>,
    bounds_requested: Option<bool>,
    generation: u64,
    bounds_generation: u64,
    resolution_count: u64,
    pending_targets: Vec<PendingTargetProjection>,
    pending_removals: Vec<PendingTargetRemoval>,
    pending_bounds: Vec<PendingTargetBounds>,
    topology_queue: VecDeque<Entity>,
    queued_topology: HashMap<Entity, bool>,
    pending_topology: Option<PendingTopologyProjection>,
}

#[derive(Debug)]
struct HierarchyCursor {
    entity: Entity,
    next_child: usize,
    entered: bool,
}

impl HierarchyCursor {
    fn new(entity: Entity) -> Self {
        Self {
            entity,
            next_child: 0,
            entered: false,
        }
    }
}

#[derive(Debug)]
struct PendingTargetProjection {
    target: SceneAnchor,
    stack: Vec<HierarchyCursor>,
    visited: HashSet<Entity>,
}

#[derive(Debug)]
struct PendingTopologyProjection {
    recurse_descendants: bool,
    processed_single: bool,
    stack: Vec<HierarchyCursor>,
    visited: HashSet<Entity>,
}

#[derive(Debug)]
struct PendingTargetRemoval {
    target: SceneAnchor,
    entities: std::collections::hash_set::IntoIter<Entity>,
}

const MAX_PROJECTION_ENTITIES_PER_UPDATE: usize = 256;

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

    fn enqueue_topology(&mut self, entity: Entity, recurse_descendants: bool) {
        if self.target_roots.is_empty() && self.pending_targets.is_empty() {
            return;
        }
        match self.queued_topology.entry(entity) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                *entry.get_mut() |= recurse_descendants;
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(recurse_descendants);
                self.topology_queue.push_back(entity);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn is_pending(&self) -> bool {
        !self.pending_targets.is_empty()
            || !self.pending_removals.is_empty()
            || !self.pending_bounds.is_empty()
            || self.pending_topology.is_some()
            || !self.topology_queue.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn pending_hierarchy_depth(&self) -> usize {
        self.pending_targets
            .iter()
            .map(|work| work.stack.len())
            .sum()
    }

    #[cfg(test)]
    pub(crate) fn pending_topology_count(&self) -> usize {
        self.topology_queue.len() + usize::from(self.pending_topology.is_some())
    }
}

/// Registers lifecycle observers that admit topology deltas into the bounded
/// projection queue. Observers avoid scanning every changed mesh/component in
/// a frame, and a `ChildOf` change explicitly reconciles its subtree.
pub(in crate::viewport) fn register_selection_projection_observers(app: &mut App) {
    app.add_observer(queue_mesh_insert)
        .add_observer(queue_mesh_remove)
        .add_observer(queue_child_of_insert)
        .add_observer(queue_child_of_remove);
}

fn queue_mesh_insert(
    event: On<Insert, Mesh3d>,
    mut projection: ResMut<SelectedRenderableProjection>,
) {
    projection.enqueue_topology(event.event_target(), false);
}

fn queue_mesh_remove(
    event: On<Remove, Mesh3d>,
    mut projection: ResMut<SelectedRenderableProjection>,
) {
    projection.enqueue_topology(event.event_target(), false);
}

fn queue_child_of_insert(
    event: On<Insert, ChildOf>,
    mut projection: ResMut<SelectedRenderableProjection>,
) {
    projection.enqueue_topology(event.event_target(), true);
}

fn queue_child_of_remove(
    event: On<Remove, ChildOf>,
    mut projection: ResMut<SelectedRenderableProjection>,
) {
    projection.enqueue_topology(event.event_target(), true);
}

#[allow(clippy::type_complexity)]
pub(in crate::viewport) fn sync_selected_renderable_projection(
    mut selection: ResMut<SelectedTargets>,
    mut selected_prim: Option<ResMut<SelectedPrim>>,
    scene_index: Res<SceneAnchorIndex>,
    mut projection: ResMut<SelectedRenderableProjection>,
    settings: Res<ViewerSettingsState>,
    policy: Option<Res<SelectionPresentationPolicy>>,
    hierarchy: Query<(Option<&Children>, Option<&Mesh3d>)>,
    geometry: Query<(
        Option<&GlobalTransform>,
        Option<&Mesh3d>,
        Option<&Aabb>,
        Option<&UsdLocalExtent>,
    )>,
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
    parent_hierarchy: Query<Option<&ChildOf>>,
) {
    if let Some(selected_prim) = selected_prim.as_deref_mut()
        && selected_prim.0.is_none()
    {
        selected_prim.0 = selection
            .0
            .primary
            .as_ref()
            .and_then(|primary| scene_index.resolve(primary));
    }

    let targets = selection.0.targets.clone();
    let bounds_requested = settings.section_box_enabled()
        || policy
            .as_deref()
            .is_some_and(|policy| policy.uses_coarse(projection.renderables.len()));
    let scene_revision = scene_index.revision();
    let scene_changed = projection.last_scene_revision != Some(scene_revision);
    let selection_changed = projection.last_selection_revision != Some(selection.revision());
    let bounds_request_changed = projection.bounds_requested != Some(bounds_requested);
    let geometry_mutated = bounds_requested && geometry_changed.iter().next().is_some();
    projection.added_renderables.clear();
    projection.removed_renderables.clear();

    if !scene_changed
        && !selection_changed
        && !geometry_mutated
        && !bounds_request_changed
        && projection.pending_targets.is_empty()
        && projection.pending_removals.is_empty()
        && projection.pending_bounds.is_empty()
        && projection.pending_topology.is_none()
        && projection.topology_queue.is_empty()
    {
        return;
    }

    let full_rebuild = projection.last_selection_revision.is_none();
    let mut mapping_changed = full_rebuild;
    let mut bounds_changed = full_rebuild || bounds_request_changed;

    if full_rebuild {
        projection.renderables.clear();
        projection.target_renderables.clear();
        projection.target_renderable_order.clear();
        projection.target_roots.clear();
        projection.roots_by_entity.clear();
        projection.renderable_targets.clear();
        projection.target_bounds.clear();
        projection.renderable_refcounts.clear();
        projection.pending_targets.clear();
        projection.pending_removals.clear();
        projection.pending_bounds.clear();
        projection.topology_queue.clear();
        projection.queued_topology.clear();
        projection.pending_topology = None;
        schedule_targets(&targets, &scene_index, &mut projection);
    } else if selection_changed || scene_changed {
        let target_mapping_changed = reconcile_targets(&targets, &scene_index, &mut projection);
        mapping_changed |= target_mapping_changed;
        bounds_changed |= target_mapping_changed && bounds_requested;
    }

    if bounds_requested && bounds_request_changed {
        for target in &targets {
            bounds_changed |= schedule_target_bounds(&mut projection, target);
        }
    }

    if bounds_requested && geometry_mutated {
        for entity in geometry_changed.iter() {
            let affected_targets = projection
                .renderable_targets
                .get(&entity)
                .cloned()
                .unwrap_or_default();
            for target in affected_targets {
                bounds_changed |= schedule_target_bounds(&mut projection, &target);
            }
        }
    }

    let (walk_changed, completed_targets) = advance_pending_targets(&mut projection, &hierarchy);
    mapping_changed |= walk_changed;
    if bounds_requested {
        for target in completed_targets {
            bounds_changed |= schedule_target_bounds(&mut projection, &target);
        }
    }

    if advance_pending_removals(&mut projection, MAX_PROJECTION_ENTITIES_PER_UPDATE) {
        mapping_changed = true;
    }

    let topology = advance_pending_topology(
        &mut projection,
        &hierarchy,
        &parent_hierarchy,
        MAX_PROJECTION_ENTITIES_PER_UPDATE,
    );
    mapping_changed |= topology.mapping_changed;
    if bounds_requested {
        for target in topology.changed_targets {
            bounds_changed |= schedule_target_bounds(&mut projection, &target);
        }
    }

    if bounds_requested {
        bounds_changed |= advance_pending_bounds(
            &mut projection,
            &geometry,
            MAX_PROJECTION_ENTITIES_PER_UPDATE,
        );
    } else {
        bounds_changed |=
            projection.aggregate_bounds.is_some() || !projection.target_bounds.is_empty();
        projection.pending_bounds.clear();
        projection.target_bounds.clear();
        projection.aggregate_bounds = None;
    }

    selection.clear_pending_delta();
    if mapping_changed {
        projection.generation = projection.generation.saturating_add(1);
    }
    if bounds_changed {
        projection.aggregate_bounds = aggregate_bounds(&projection.target_bounds);
        projection.bounds_generation = projection.bounds_generation.saturating_add(1);
    }
    projection.last_selection_revision = Some(selection.revision());
    projection.last_scene_revision = Some(scene_revision);
    projection.bounds_requested = Some(bounds_requested);
}
