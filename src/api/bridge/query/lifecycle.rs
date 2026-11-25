use bevy::prelude::*;
use viewport_protocol::{HierarchySource, ViewportEvent, ViewportEventEnvelope};

use super::super::ViewerSettingsState;
use super::super::helpers::build_read_model;
use crate::animation::UsdStageTime;
use crate::api::ViewportEventOutbox;
use crate::bim::ClassificationColorPlan;
use crate::viewport::navigation::camera::{CameraMount, CameraOrientationState};
use crate::physics::PhysicsActive;
use crate::viewport::rendering::scene::SelectedTargets;
use crate::viewport::rendering::scene::visualization::DisplayToggles;
use crate::viewport::rendering::scene::{
    ActiveHierarchyProvider, CurrentHierarchyProjection, SceneAnchorIndex,
    refresh_projection_visibility,
};
use crate::viewport::stage::{LoaderTuning, Spawned, StageHandle, StageInfo};

/// Rebuilds the active virtual provider only when the semantic snapshot
/// changes. Prim projection refresh remains owned by `SceneAnchorIndex`.
pub(crate) fn refresh_active_hierarchy_projection(
    provider: Res<ActiveHierarchyProvider>,
    semantic: Res<crate::semantic::viewport::SemanticSyncState>,
    scene_index: Res<SceneAnchorIndex>,
    mut current_projection: ResMut<CurrentHierarchyProjection>,
    mut color_plan: Option<ResMut<ClassificationColorPlan>>,
) {
    if provider.source() != HierarchySource::BimClassification
        || (!semantic.is_changed() && !provider.is_changed() && !scene_index.is_changed())
    {
        return;
    }
    let (Some(recipe), Some(snapshot), Some(index)) = (
        provider.classification_recipe(),
        semantic.snapshot(),
        semantic.shared_bim_index(),
    ) else {
        return;
    };
    let mut service = crate::bim::BimReadService::with_index(snapshot, index);
    let color_intent = color_plan.as_ref().and_then(|plan| plan.intent());
    let color_entries = color_intent.as_ref().map(|intent| {
        match service.classification_color_entries(recipe, intent) {
            Ok(entries) => entries,
            Err(error) => {
                bevy::log::warn!(
                    error = %error,
                    "classification color intent could not be materialized"
                );
                Vec::new()
            }
        }
    });
    let Ok(mut projection) = service.classification_projection(recipe) else {
        return;
    };
    refresh_projection_visibility(&mut projection, &scene_index);
    *current_projection = projection;
    if let (Some(plan), Some(entries)) = (color_plan.as_deref_mut(), color_entries) {
        plan.replace_entries(entries);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublishedStageKey {
    state: viewport_protocol::StageLoadState,
    activation_generation: u64,
    hierarchy_source: HierarchySource,
    hierarchy_revision: u64,
    scene_revision: u64,
}

/// Emits lifecycle changes independently of who initiated the load. That
/// makes manual reloads, file-watcher reloads, and future host commands all
/// observable through the same public event.
#[allow(clippy::too_many_arguments)]
pub(crate) fn publish_stage_load_state(
    stage: Option<Res<StageHandle>>,
    spawned: Res<Spawned>,
    stage_info: Res<StageInfo>,
    selection: Res<SelectedTargets>,
    viewer_settings: Res<ViewerSettingsState>,
    (scene_index, current_projection): (
        Res<SceneAnchorIndex>,
        Option<Res<CurrentHierarchyProjection>>,
    ),
    camera_mount: Res<CameraMount>,
    camera_orientation: Res<CameraOrientationState>,
    clock: Res<UsdStageTime>,
    toggles: Res<DisplayToggles>,
    tuning: Res<LoaderTuning>,
    physics: Res<PhysicsActive>,
    gate: crate::viewport::navigation::input::ViewerInteractionGate,
    visual_health: Option<Res<crate::viewport::rendering::residency::VisualStreamingHealth>>,
    mut last: Local<Option<PublishedStageKey>>,
    mut outbox: ResMut<ViewportEventOutbox>,
) {
    use viewport_protocol::StageLoadState;
    let initial_hierarchy_pending =
        scene_index.has_pending_initial_publication(stage_info.activation_generation);
    let state = match stage {
        None => StageLoadState::Idle,
        Some(stage) => match stage
            .error
            .as_deref()
            .or_else(|| gate.projection_error())
            .or_else(|| {
                visual_health
                    .as_deref()
                    .and_then(|health| health.last_error.as_deref().or(health.lod_error.as_deref()))
            }) {
            Some(error) => StageLoadState::Failed {
                message: error.to_owned(),
            },
            None if spawned.0
                && !gate.locked()
                && !initial_hierarchy_pending
                && !visual_health
                    .as_deref()
                    .is_some_and(|health| health.awaiting_payloads) =>
            {
                StageLoadState::Ready
            }
            _ => StageLoadState::Loading,
        },
    };
    let current_key = PublishedStageKey {
        state: state.clone(),
        activation_generation: stage_info.activation_generation,
        hierarchy_source: current_projection
            .as_deref()
            .map_or(HierarchySource::Prim, CurrentHierarchyProjection::source),
        hierarchy_revision: current_projection
            .as_deref()
            .map_or(0, CurrentHierarchyProjection::revision),
        scene_revision: scene_index.revision(),
    };
    let state_changed = last
        .as_ref()
        .is_none_or(|previous| previous.state != current_key.state);
    let generation_changed = last.as_ref().is_none_or(|previous| {
        previous.activation_generation != current_key.activation_generation
    });
    let hierarchy_changed = last.as_ref().is_none_or(|previous| {
        previous.hierarchy_source != current_key.hierarchy_source
            || previous.hierarchy_revision != current_key.hierarchy_revision
    });
    let scene_changed = last
        .as_ref()
        .is_none_or(|previous| previous.scene_revision != current_key.scene_revision);
    let should_publish = state_changed
        || generation_changed
        || (matches!(state, StageLoadState::Ready) && (hierarchy_changed || scene_changed))
        || (matches!(state, StageLoadState::Loading) && hierarchy_changed);
    if should_publish {
        if state_changed {
            outbox.push(ViewportEventEnvelope::new(
                None,
                ViewportEvent::StageLoadStateChanged {
                    state: state.clone(),
                },
            ));
        }
        let snapshot = build_read_model(
            &stage_info,
            spawned.0 && matches!(state, StageLoadState::Ready),
            &selection.0,
            selection.revision(),
            &viewer_settings.0,
            &scene_index,
            current_projection.as_deref(),
            &camera_mount,
            &camera_orientation.latest,
            &clock,
            &toggles,
            &tuning,
            physics.0,
        );
        info!(
            "[viewport-scene] publishing {:?} snapshot: generation={} source={:?} hierarchy_rev={} scene_rev={} total_prims={} total_roots={} payload_prims={}",
            state,
            current_key.activation_generation,
            current_key.hierarchy_source,
            current_key.hierarchy_revision,
            current_key.scene_revision,
            snapshot.scene.total_prims,
            snapshot.scene.total_roots,
            snapshot.scene.prims.len()
        );
        outbox.push(ViewportEventEnvelope::new(
            None,
            ViewportEvent::Snapshot {
                state: Box::new(snapshot),
            },
        ));
        *last = Some(current_key);
    }
}
