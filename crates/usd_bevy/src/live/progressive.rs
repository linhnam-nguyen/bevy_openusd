use bevy::mesh::Mesh3d;
use bevy::prelude::*;
use std::time::Instant;

use super::animation::{AnimatedPrims, prim_is_animated};
use super::index::PrimEntities;
use super::progressive_cleanup::clear_projection;
use super::progressive_resident::resident_projection;
use super::progressive_state::{ProgressiveProjectionState, ProjectionBudget, ProjectionReadiness};
use super::projection::{ProjectionStats, project_plan_entry, registry_of};
use super::projection_plan::ProjectionPlanBuilder;
use super::stage::LiveStage;

/// The exclusive system that owns initial projection planning and draining.
pub(super) fn project_on_load_system(world: &mut World) {
    let Some((session_id, has_changes)) = world
        .get_non_send::<LiveStage>()
        .map(|live| (live.session_id(), live.has_changes()))
    else {
        return;
    };
    let (state_session, readiness, restart_requested, total, validate_resident) = {
        let state = world.resource::<ProgressiveProjectionState>();
        (
            state.session_id,
            state.readiness,
            state.restart_requested,
            state.total,
            state.resident_validation_requested,
        )
    };
    let map_len = world.resource::<PrimEntities>().len();
    if matches!(
        readiness,
        ProjectionReadiness::Planning | ProjectionReadiness::Projecting
    ) && has_changes
    {
        world
            .resource_mut::<ProgressiveProjectionState>()
            .restart_requested = true;
        return;
    }
    let resident = state_session == Some(session_id)
        && readiness == ProjectionReadiness::Ready
        && total == map_len
        && (!validate_resident
            || resident_projection(
                world,
                world.resource::<PrimEntities>(),
                world.resource::<ProgressiveProjectionState>(),
            ));
    if resident {
        let mut state = world.resource_mut::<ProgressiveProjectionState>();
        state.resident_short_circuits += 1;
        state.resident_validation_requested = false;
        return;
    }
    if state_session == Some(session_id)
        && matches!(
            readiness,
            ProjectionReadiness::Planning | ProjectionReadiness::Projecting
        )
        && !has_changes
        && !restart_requested
    {
        let Some(live) = world.remove_non_send::<LiveStage>() else {
            return;
        };
        let mut map = world.remove_resource::<PrimEntities>().unwrap_or_default();
        drain_generation(world, &live, &mut map);
        world.insert_resource(map);
        world.insert_non_send(live);
        return;
    }
    let needs_start = state_session != Some(session_id)
        || restart_requested
        || (readiness == ProjectionReadiness::Idle && map_len == 0)
        || (readiness == ProjectionReadiness::Cancelled && !has_changes)
        || (readiness == ProjectionReadiness::Ready && !resident);
    if !needs_start || has_changes {
        return;
    }

    let Some(live) = world.remove_non_send::<LiveStage>() else {
        return;
    };
    let mut map = world.remove_resource::<PrimEntities>().unwrap_or_default();
    start_generation(world, &live, &mut map, session_id);
    drain_generation(world, &live, &mut map);
    world.insert_resource(map);
    world.insert_non_send(live);
}

fn start_generation(world: &mut World, live: &LiveStage, map: &mut PrimEntities, session_id: u64) {
    let should_clear = world
        .resource::<ProgressiveProjectionState>()
        .session_id
        .is_some_and(|current| current != session_id)
        || !map.is_empty();
    if should_clear {
        clear_projection(world, map);
    }

    let generation_started = Instant::now();
    let plan_builder = ProjectionPlanBuilder::new(&live.stage);
    world.remove_non_send::<ProjectionPlanBuilder>();
    world.insert_non_send(plan_builder);
    world.insert_resource(AnimatedPrims::default());
    let mut state = world.resource_mut::<ProgressiveProjectionState>();
    state.readiness = ProjectionReadiness::Planning;
    state.plan_builds += 1;
    state.last_error = None;
    state.generation = state
        .generation
        .checked_add(1)
        .expect("projection generation exhausted");
    state.session_id = Some(session_id);
    state.completed = 0;
    state.total = 0;
    state.plan = None;
    state.next_index = 0;
    state.entities = vec![None];
    state.animated.clear();
    state.started_at = Some(generation_started);
    state.planning_ms = None;
    state.planning_updates = 0;
    state.planning_work_items = 0;
    state.first_projected_prim_ms = None;
    state.first_mesh_ms = None;
    state.restart_requested = false;
    state.resident_validation_requested = false;
    world.insert_resource(ProjectionStats::default());
}

fn drain_generation(world: &mut World, live: &LiveStage, map: &mut PrimEntities) {
    let budget = *world.resource::<ProjectionBudget>();
    let started = Instant::now();
    let mut processed = 0usize;
    let registry = registry_of(world);
    if world.contains_non_send::<ProjectionPlanBuilder>() {
        world
            .resource_mut::<ProgressiveProjectionState>()
            .planning_updates += 1;
    }
    loop {
        if budget
            .max_work_items
            .is_some_and(|limit| processed >= limit)
            || budget
                .max_duration
                .is_some_and(|limit| started.elapsed() >= limit)
        {
            break;
        }
        let plan_is_finished = world
            .get_non_send::<ProjectionPlanBuilder>()
            .is_some_and(ProjectionPlanBuilder::is_finished);
        if plan_is_finished {
            finalize_plan(world);
            continue;
        }
        let needs_plan_work = {
            let next_index = world.resource::<ProgressiveProjectionState>().next_index;
            world
                .get_non_send::<ProjectionPlanBuilder>()
                .is_some_and(|builder| next_index >= builder.len())
        };
        if needs_plan_work {
            let result = world
                .get_non_send_mut::<ProjectionPlanBuilder>()
                .expect("planning state exists")
                .advance_one();
            processed += 1;
            match result {
                Ok(_) => {
                    let discovered = world
                        .get_non_send::<ProjectionPlanBuilder>()
                        .map_or(0, ProjectionPlanBuilder::len);
                    let mut state = world.resource_mut::<ProgressiveProjectionState>();
                    state.planning_work_items += 1;
                    state.entities.resize(discovered, None);
                }
                Err(error) => {
                    fail_generation(world, error);
                    break;
                }
            }
            continue;
        }
        let (index, entry, parent) = {
            let state = world.resource::<ProgressiveProjectionState>();
            let entry = world
                .get_non_send::<ProjectionPlanBuilder>()
                .and_then(|builder| builder.entry(state.next_index))
                .or_else(|| {
                    state
                        .plan
                        .as_ref()
                        .and_then(|plan| plan.entry(state.next_index))
                })
                .cloned();
            let Some(entry) = entry else {
                break;
            };
            let parent = entry
                .parent_index()
                .and_then(|parent| state.entities.get(parent).copied().flatten());
            (state.next_index, entry, parent)
        };
        let entity = project_plan_entry(world, &live.stage, &registry, map, &entry, parent);
        let is_mesh = world.get::<Mesh3d>(entity).is_some();
        if entry.path() != "/"
            && openusd::sdf::path(entry.path())
                .ok()
                .is_some_and(|path| prim_is_animated(&live.stage, &path))
        {
            world
                .resource_mut::<ProgressiveProjectionState>()
                .animated
                .insert(entry.path().to_string());
        }
        let elapsed_ms = world
            .resource::<ProgressiveProjectionState>()
            .started_at
            .map(|start| start.elapsed().as_secs_f64() * 1000.0);
        let mut state = world.resource_mut::<ProgressiveProjectionState>();
        state.entities[index] = Some(entity);
        state.next_index += 1;
        state.completed += 1;
        if entry.path() != "/" && state.first_projected_prim_ms.is_none() {
            state.first_projected_prim_ms = elapsed_ms;
        }
        if is_mesh && state.first_mesh_ms.is_none() {
            state.first_mesh_ms = elapsed_ms;
        }
        processed += 1;
    }
    finish_if_ready(world, live);
}

fn finalize_plan(world: &mut World) {
    let builder = world
        .remove_non_send::<ProjectionPlanBuilder>()
        .expect("finished projection planner exists");
    let plan = builder.finish().expect("finished planner produces a plan");
    let mut state = world.resource_mut::<ProgressiveProjectionState>();
    state.total = plan.len();
    state.plan = Some(plan);
    state.readiness = ProjectionReadiness::Projecting;
    state.planning_ms = state
        .started_at
        .map(|start| start.elapsed().as_secs_f64() * 1000.0);
}

fn fail_generation(world: &mut World, error: anyhow::Error) {
    let mut state = world.resource_mut::<ProgressiveProjectionState>();
    state.readiness = ProjectionReadiness::Failed;
    state.last_error = Some(format!(
        "failed to build deterministic projection plan: {error:#}"
    ));
    world.remove_non_send::<ProjectionPlanBuilder>();
}

fn finish_if_ready(world: &mut World, live: &LiveStage) {
    let (ready, animated, duration_ms, prims) = {
        let mut state = world.resource_mut::<ProgressiveProjectionState>();
        let complete = state.total > 0 && state.completed == state.total;
        if !complete {
            return;
        }
        state.readiness = ProjectionReadiness::Ready;
        let duration_ms = state
            .started_at
            .map(|start| start.elapsed().as_secs_f64() * 1000.0);
        (
            true,
            std::mem::take(&mut state.animated),
            duration_ms,
            state.total.saturating_sub(1),
        )
    };
    if ready {
        world.insert_resource(AnimatedPrims(animated));
        if let Some(mut stats) = world.get_resource_mut::<ProjectionStats>() {
            stats.initial_projection_ms = duration_ms;
            stats.initial_projection_prims = prims as u64;
        }
        bevy::log::info!(
            session = live.session_id(),
            prims,
            duration_ms = duration_ms.unwrap_or_default(),
            "progressive USD stage projection ready"
        );
    }
}
