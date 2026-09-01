use bevy::prelude::*;
use viewport_protocol::{HierarchySource, ViewportEvent, ViewportEventEnvelope};

use super::super::ViewerSettingsState;
use super::super::helpers::build_read_model;
use crate::viewport::animation::UsdStageTime;
use crate::viewport::api::{
    ActiveHierarchyProvider, CurrentHierarchyProjection, SceneAnchorIndex, ViewportEventOutbox,
    refresh_projection_visibility,
};
use crate::viewport::camera::{CameraMount, CameraOrientationState};
use crate::viewport::physics::PhysicsActive;
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::scene::{ClassificationColorPlan, SelectedTargets};
use crate::viewport::session::{LoaderTuning, Spawned, StageHandle, StageInfo};

/// Rebuilds the active virtual provider only when the semantic snapshot
/// changes. Prim projection refresh remains owned by `SceneAnchorIndex`.
pub(crate) fn refresh_active_hierarchy_projection(
    provider: Res<ActiveHierarchyProvider>,
    semantic: Res<crate::viewport::semantic::SemanticSyncState>,
    scene_index: Res<SceneAnchorIndex>,
    mut current_projection: ResMut<CurrentHierarchyProjection>,
    mut color_plan: Option<ResMut<ClassificationColorPlan>>,
) {
    if provider.source() != HierarchySource::BimClassification || !semantic.is_changed() {
        return;
    }
    let (Some(recipe), Some(snapshot), Some(index)) = (
        provider.classification_recipe(),
        semantic.snapshot(),
        semantic.shared_bim_index(),
    ) else {
        return;
    };
    let mut service = crate::viewport::bim::BimReadService::with_index(snapshot, index);
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
    scene_index: Res<SceneAnchorIndex>,
    camera_mount: Res<CameraMount>,
    camera_orientation: Res<CameraOrientationState>,
    clock: Res<UsdStageTime>,
    toggles: Res<DisplayToggles>,
    tuning: Res<LoaderTuning>,
    physics: Res<PhysicsActive>,
    mut last: Local<Option<(viewport_protocol::StageLoadState, u64)>>,
    mut outbox: ResMut<ViewportEventOutbox>,
) {
    use viewport_protocol::StageLoadState;
    let state = match stage {
        None => StageLoadState::Idle,
        Some(stage) => match &stage.error {
            Some(error) => StageLoadState::Failed {
                message: error.clone(),
            },
            None if spawned.0 => StageLoadState::Ready,
            _ => StageLoadState::Loading,
        },
    };
    let state_changed = last.as_ref().is_none_or(|(previous, _)| previous != &state);
    let scene_changed = last
        .as_ref()
        .is_none_or(|(_, revision)| *revision != scene_index.revision());
    if state_changed || (matches!(state, StageLoadState::Ready) && scene_changed) {
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
            &camera_mount,
            &camera_orientation.latest,
            &clock,
            &toggles,
            &tuning,
            physics.0,
        );
        info!(
            "[viewport-scene] publishing {:?} snapshot: total_prims={} total_roots={} payload_prims={}",
            state,
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
        *last = Some((state, scene_index.revision()));
    }
}
