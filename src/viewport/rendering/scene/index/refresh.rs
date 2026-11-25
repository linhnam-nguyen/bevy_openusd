//! Bounded Scene-index admission and reconciliation scheduling.
//!
//! Lifecycle observers coalesce mutations into an authority state. The update
//! system performs at most `SCENE_INDEX_ADMISSION_BUDGET` row captures or
//! additive admissions before yielding.

use bevy::ecs::hierarchy::{ChildOf, Children};
use bevy::ecs::lifecycle::{Insert, Remove};
use bevy::ecs::observer::On;
use bevy::prelude::*;
use usd_bevy::{UsdDisplayName, UsdHierarchyTarget, UsdPrimRef, UsdTransparentHierarchyNode};

use super::super::hierarchy::CurrentHierarchyProjection;
use super::{SCENE_INDEX_ADMISSION_BUDGET, SceneAnchorIndex};
use crate::viewport::stage::{
    SceneCachePresentation, Spawned, StageInfo, StagePresentationContext,
};

#[derive(Component)]
pub(crate) struct SceneIndexIndexedPrim;

pub(crate) fn register_scene_index_observers(app: &mut App) {
    app.add_observer(queue_scene_prim_insert)
        .add_observer(queue_scene_prim_remove)
        .add_observer(super::occurrence::upsert_point_occurrence)
        .add_observer(super::occurrence::remove_point_occurrence)
        .add_observer(queue_display_name_remove)
        .add_observer(queue_hierarchy_target_remove)
        .add_observer(queue_transparent_remove)
        .add_observer(queue_visibility_remove)
        .add_observer(queue_child_of_remove);
}

fn queue_scene_prim_insert(
    event: On<Insert<UsdPrimRef>>,
    prims: Query<&UsdPrimRef>,
    mut index: ResMut<SceneAnchorIndex>,
) {
    let entity = event.entity;
    if prims.get(entity).is_ok_and(|prim| prim.path == "/") {
        return;
    }
    if index.reconcile.contains(entity) {
        index.reconcile.request();
    } else if index.queued_additions.insert(entity) {
        index.pending_additions.push_back(entity);
    }
}

fn queue_scene_prim_remove(event: On<Remove<UsdPrimRef>>, mut index: ResMut<SceneAnchorIndex>) {
    let entity = event.entity;
    index.queued_additions.remove(&entity);
    index.reconcile.remove(entity);
}

fn request_if_indexed(entity: Entity, index: &mut SceneAnchorIndex) {
    if index.reconcile.contains(entity) {
        index.reconcile.request();
    }
}

fn queue_display_name_remove(
    event: On<Remove<UsdDisplayName>>,
    mut index: ResMut<SceneAnchorIndex>,
) {
    request_if_indexed(event.entity, &mut index);
}

fn queue_hierarchy_target_remove(
    event: On<Remove<UsdHierarchyTarget>>,
    mut index: ResMut<SceneAnchorIndex>,
) {
    request_if_indexed(event.entity, &mut index);
}

fn queue_transparent_remove(
    event: On<Remove<UsdTransparentHierarchyNode>>,
    mut index: ResMut<SceneAnchorIndex>,
) {
    request_if_indexed(event.entity, &mut index);
}

fn queue_visibility_remove(event: On<Remove<Visibility>>, mut index: ResMut<SceneAnchorIndex>) {
    request_if_indexed(event.entity, &mut index);
}

fn queue_child_of_remove(event: On<Remove<ChildOf>>, mut index: ResMut<SceneAnchorIndex>) {
    request_if_indexed(event.entity, &mut index);
}

fn projection_is_settled(state: Option<&usd_bevy::ProgressiveProjectionState>) -> bool {
    state.is_none_or(|state| {
        !matches!(
            state.readiness(),
            usd_bevy::ProjectionReadiness::Planning | usd_bevy::ProjectionReadiness::Projecting
        )
    })
}

#[allow(clippy::type_complexity)]
pub(crate) fn refresh_scene_anchor_index(
    _spawned: Res<Spawned>,
    mut commands: Commands,
    changed_prims: Query<
        Entity,
        (
            With<UsdPrimRef>,
            With<SceneIndexIndexedPrim>,
            Or<(
                Changed<UsdPrimRef>,
                Changed<UsdDisplayName>,
                Added<UsdHierarchyTarget>,
                Changed<UsdHierarchyTarget>,
                Added<UsdTransparentHierarchyNode>,
                Changed<UsdTransparentHierarchyNode>,
                Changed<Visibility>,
            )>,
        ),
    >,
    hierarchy_changed: Query<
        Entity,
        (
            With<UsdPrimRef>,
            With<SceneIndexIndexedPrim>,
            Or<(Added<ChildOf>, Changed<ChildOf>)>,
        ),
    >,
    unindexed_prims: Query<Entity, (With<UsdPrimRef>, Without<SceneIndexIndexedPrim>)>,
    prims: Query<(
        Entity,
        &UsdPrimRef,
        Option<&UsdDisplayName>,
        Option<&UsdHierarchyTarget>,
        Option<&UsdTransparentHierarchyNode>,
        Option<&Visibility>,
        Option<&Children>,
    )>,
    parent_hierarchy: Query<Option<&ChildOf>>,
    mut index: ResMut<SceneAnchorIndex>,
    mut current_projection: ResMut<CurrentHierarchyProjection>,
    provider: Option<Res<super::super::ActiveHierarchyProvider>>,
    presentation: Option<Res<StagePresentationContext>>,
    stage_info: Option<Res<StageInfo>>,
    scene_cache: Option<Res<SceneCachePresentation>>,
    progressive: Option<Res<usd_bevy::ProgressiveProjectionState>>,
) {
    index.last_refresh_admitted = 0;

    let active_generation = stage_info.as_deref().map(|info| info.activation_generation);
    if index.active_activation_generation != active_generation {
        index.active_activation_generation = active_generation;
        index.published_activation_generation = None;
    }

    let presentation_changed = presentation
        .as_ref()
        .is_some_and(|presentation| presentation.is_changed());
    if presentation_changed
        || changed_prims.iter().next().is_some()
        || hierarchy_changed.iter().next().is_some()
    {
        index.reconcile.request();
    }

    if index.reconcile.is_pending() {
        if let Some(projection) = index.advance_full_reconciliation(
            &prims,
            &parent_hierarchy,
            presentation.as_deref(),
            SCENE_INDEX_ADMISSION_BUDGET,
        ) {
            index.published_activation_generation = active_generation;
            if provider.as_ref().is_none_or(|provider| {
                provider.source() == viewport_protocol::HierarchySource::Prim
            }) {
                *current_projection = projection;
            }
            info!(
                "[viewport-scene-index] published bounded reconciliation published_revision={} prims={}",
                index.published_revision,
                index.nodes.len(),
            );
        }
        return;
    }

    if index.pending_additions.is_empty() {
        for entity in unindexed_prims
            .iter()
            .filter(|entity| {
                prims
                    .get(*entity)
                    .is_ok_and(|(_, prim, ..)| prim.path != "/")
            })
            .take(SCENE_INDEX_ADMISSION_BUDGET)
        {
            if index.queued_additions.insert(entity) {
                index.pending_additions.push_back(entity);
            }
        }
    }

    if !index.initialized && index.pending_additions.is_empty() {
        index.initialized = true;
    }

    if !index.pending_additions.is_empty() {
        let mut batch = Vec::with_capacity(SCENE_INDEX_ADMISSION_BUDGET);
        for _ in 0..SCENE_INDEX_ADMISSION_BUDGET {
            let Some(entity) = index.pending_additions.pop_front() else {
                break;
            };
            index.queued_additions.remove(&entity);
            if prims.get(entity).is_ok() {
                batch.push(entity);
            }
        }

        if batch.is_empty() {
            // Stale queue entries consume only this bounded dequeue update.
            return;
        }

        match index.ingest_added(
            batch.iter().copied(),
            &prims,
            &parent_hierarchy,
            presentation.as_deref(),
        ) {
            Some(admitted) => {
                index.last_refresh_admitted = admitted;
                index.initialized = true;
                for entity in batch {
                    index.reconcile.admit(entity);
                    commands.entity(entity).insert(SceneIndexIndexedPrim);
                }
                info!(
                    "[viewport-scene-index] admitted progressive admission_revision={} published_revision={} admitted={} pending={} prims={}",
                    index.admission_revision,
                    index.published_revision,
                    admitted,
                    index.pending_additions.len(),
                    index.nodes.len(),
                );
            }
            None => {
                // Duplicate/native-instance identity cannot be represented by
                // the unique incremental map. Admit this bounded batch to the
                // reconciliation membership and defer whole-index rebuilding;
                // never escape into `prims.iter()` from this update.
                for entity in batch {
                    index.reconcile.admit(entity);
                    commands.entity(entity).insert(SceneIndexIndexedPrim);
                }
                index.reconcile.request();
            }
        }
        return;
    }

    // Startup intentionally keeps the public projection coherent: partial
    // authority admission remains private until the bounded queue drains and
    // the coalesced derived view is published below. An empty final activation
    // still publishes one observable revision for its generation when no cache
    // bootstrap owns the interim presentation.

    let needs_empty_final_publish = active_generation.is_some()
        && index.published_activation_generation != active_generation
        && scene_cache.is_none();
    if (index.derived_dirty || needs_empty_final_publish)
        && projection_is_settled(progressive.as_deref())
    {
        let projection = index.flush_incremental_derived();
        index.published_activation_generation = active_generation;
        if provider
            .as_ref()
            .is_none_or(|provider| provider.source() == viewport_protocol::HierarchySource::Prim)
        {
            *current_projection = projection;
        }
        info!(
            "[viewport-scene-index] published coalesced published_revision={} prims={}",
            index.published_revision,
            index.nodes.len(),
        );
    }
}
