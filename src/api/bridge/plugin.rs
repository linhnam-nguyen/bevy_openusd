use bevy::prelude::*;
use usd_bevy::LiveStage;
use viewport_protocol::{PROTOCOL_VERSION, ViewportEvent, ViewportEventEnvelope};

use super::ViewerSettingsState;
use super::bim::properties_lifecycle::{PendingBimProperties, replay_pending_bim_properties};
use super::commands::{
    apply_pending_renderer_cadence, apply_viewport_commands, reject_locked_viewport_commands,
};
use super::query::{
    dispatch_scene_query_commands, publish_scene_query_results, publish_stage_load_state,
    refresh_active_hierarchy_projection,
};
use super::state::{
    EditorHistories, RuntimeMutationCoordinator, SceneSearchRequests, ViewportBridgeSet,
};
use super::tree::apply_tree_commands;
use crate::cache::historical_geometry::HistoricalGeometryCache;
use crate::document::stages::recovery::worker::{RecoveryRuntime, drain_recovery_results};
use crate::document::stages::recovery::{RecoveryCheckpointWork, RecoverySettings};
use crate::api::{
    ViewportCommandInbox, ViewportEventOutbox, ViewportReadModelState, ViewportTreeCommandInbox,
};
use crate::bim::{
    BimClassificationFieldCatalogueState, BimProvenanceService, ClassificationColorPlan,
};
use crate::viewport::rendering::scene::SelectedTargets;
use crate::viewport::rendering::scene::{
    ActiveHierarchyProvider, BimClassificationRecipeState, CurrentHierarchyProjection,
    SceneAnchorIndex,
};
use crate::semantic::viewport::{
    RuntimeDeliveryRuntime, SemanticDiffState, SemanticSyncState, SemanticWorkingStore,
    drain_runtime_delivery_results, flush_pending_runtime_delivery, synchronize_live_stage,
};
use crate::viewport::stage::StageInfo;
use crate::viewport::stage::rehydrate_activation_presentation;

/// Installs the in-process implementation of the public viewport contract.
pub(crate) struct ViewportBridgePlugin;

impl Plugin for ViewportBridgePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ViewportCommandInbox>()
            .init_resource::<ViewportTreeCommandInbox>()
            .init_resource::<ViewportEventOutbox>()
            .init_resource::<ViewportReadModelState>()
            .init_resource::<SceneAnchorIndex>()
            .init_resource::<ActiveHierarchyProvider>()
            .init_resource::<BimClassificationRecipeState>()
            .init_resource::<CurrentHierarchyProjection>()
            .init_resource::<crate::api::query::SceneQueryService>()
            .init_resource::<BimProvenanceService>()
            .init_resource::<BimClassificationFieldCatalogueState>()
            .init_resource::<SelectedTargets>()
            .init_resource::<ClassificationColorPlan>()
            .init_resource::<ViewerSettingsState>()
            .init_resource::<SemanticWorkingStore>()
            .init_resource::<RuntimeDeliveryRuntime>()
            .insert_resource(product_stage_lifecycle_hooks())
            .init_resource::<SemanticSyncState>()
            .init_resource::<SemanticDiffState>()
            .init_resource::<SceneSearchRequests>()
            .init_resource::<PendingBimProperties>()
            .init_resource::<EditorHistories>()
            .init_resource::<RuntimeMutationCoordinator>()
            .init_resource::<RecoverySettings>()
            .init_resource::<RecoveryRuntime>()
            .init_resource::<HistoricalGeometryCache>()
            .add_systems(Startup, emit_viewport_ready)
            .configure_sets(
                Update,
                (
                    ViewportBridgeSet::RefreshSceneIndex,
                    ViewportBridgeSet::ApplyCommands
                        .in_set(crate::viewport::ViewportFrameSet::ApplyCommands),
                    ViewportBridgeSet::ApplyTreeCommands
                        .in_set(crate::viewport::ViewportFrameSet::ApplyTreeCommands),
                    ViewportBridgeSet::PublishStageLoadState
                        .in_set(crate::viewport::ViewportFrameSet::PublishStageLoadState),
                    ViewportBridgeSet::ReduceEvents
                        .in_set(crate::viewport::ViewportFrameSet::ReduceEvents),
                )
                    .chain(),
            )
            .add_systems(
                Update,
                crate::viewport::rendering::scene::index::refresh_scene_anchor_index
                    .in_set(ViewportBridgeSet::RefreshSceneIndex),
            )
            .add_systems(
                Update,
                (
                    rehydrate_activation_presentation,
                    replay_pending_bim_properties,
                )
                    .chain()
                    .after(crate::viewport::rendering::scene::index::refresh_scene_anchor_index)
                    .in_set(ViewportBridgeSet::RefreshSceneIndex),
            )
            // LiveStagePlugin drains and reprojects in Update. PostUpdate is
            // the first schedule where the retained batch is guaranteed to
            // represent the completed current-frame revision.
            .add_systems(
                PostUpdate,
                (
                    drain_runtime_delivery_results,
                    synchronize_live_stage,
                    refresh_active_hierarchy_projection,
                    flush_pending_runtime_delivery,
                    drain_recovery_results,
                    checkpoint_recovery,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    reject_locked_viewport_commands,
                    publish_scene_query_results,
                    dispatch_scene_query_commands,
                    apply_viewport_commands,
                )
                    .chain()
                    .in_set(ViewportBridgeSet::ApplyCommands),
            )
            .add_systems(
                Update,
                apply_pending_renderer_cadence.after(ViewportBridgeSet::ApplyCommands),
            )
            .add_systems(
                Update,
                apply_tree_commands.in_set(ViewportBridgeSet::ApplyTreeCommands),
            )
            .add_systems(
                Update,
                publish_stage_load_state.in_set(ViewportBridgeSet::PublishStageLoadState),
            )
            .add_systems(
                Update,
                reduce_authoritative_events.in_set(ViewportBridgeSet::ReduceEvents),
            );
        crate::viewport::rendering::scene::index::register_scene_index_observers(app);
    }
}

/// Exports one scratch checkpoint after the authoritative change fan-out has
/// completed for the frame. Only CPU serialization and owned-data submission
/// happen here; the recovery worker performs all filesystem operations.
pub(super) fn checkpoint_recovery(
    settings: Res<RecoverySettings>,
    runtime: Res<RecoveryRuntime>,
    pending: Res<usd_bevy::PendingStageChanges>,
    stage: Option<NonSend<LiveStage>>,
    stage_info: Res<StageInfo>,
    mut counters: Option<ResMut<crate::diagnostics::performance::RendererCounters>>,
) {
    if pending.batch().is_none() {
        return;
    }
    let Some(stage) = stage else {
        return;
    };
    if stage_info.path.trim().is_empty() {
        return;
    }

    let serialize_started = std::time::Instant::now();
    let stage_bytes = match usd_bevy::authoring::export_stage_string(&stage.stage) {
        Ok(stage) => stage.into_bytes(),
        Err(error) => {
            bevy::log::error!("[recovery] cannot serialize checkpoint: {error:#}");
            return;
        }
    };
    if let Some(ref mut counters) = counters {
        counters.recovery_serialize_ms += serialize_started.elapsed().as_secs_f64() * 1000.0;
    }
    let work = RecoveryCheckpointWork {
        project_root: settings.project_root.clone(),
        session_id: stage.session_id(),
        live_revision: pending
            .batch()
            .map(|batch| batch.revision.0)
            .unwrap_or_else(|| stage.current_revision().0),
        source_stage: stage_info.path.clone(),
        base_revision: None,
        stage_bytes,
    };
    let submit_started = std::time::Instant::now();
    if runtime.submit(work) {
        if let Some(ref mut counters) = counters {
            counters.recovery_submit_ms += submit_started.elapsed().as_secs_f64() * 1000.0;
            counters.recovery_checkpoints += 1;
            let (pending, high_water, coalesced) = runtime.stats();
            counters.recovery_mailbox_pending = pending;
            counters.recovery_mailbox_high_water =
                counters.recovery_mailbox_high_water.max(high_water);
            counters.recovery_mailbox_coalesced = coalesced;
        }
    } else {
        bevy::log::error!("[recovery] checkpoint worker queue is closed");
    }
}

/// Reduces each newly emitted authoritative event for the local Frost
/// reference adapter. The transport-delivery queue remains untouched.
fn reduce_authoritative_events(
    mut outbox: ResMut<ViewportEventOutbox>,
    mut read_model: ResMut<ViewportReadModelState>,
) {
    for event in outbox.take_published() {
        read_model.apply(&event);
    }
}

fn emit_viewport_ready(mut outbox: ResMut<ViewportEventOutbox>) {
    outbox.push(ViewportEventEnvelope::new(
        None,
        ViewportEvent::Ready {
            protocol_version: PROTOCOL_VERSION,
        },
    ));
}

pub(crate) fn product_stage_lifecycle_hooks() -> crate::viewport::stage::ProductStageLifecycleHooks {
    crate::viewport::stage::ProductStageLifecycleHooks {
        on_reset_derived: |world, activation_generation| {
            if let Some(mut semantic) =
                world.get_resource_mut::<crate::semantic::viewport::SemanticSyncState>()
            {
                semantic.reset_for_activation(activation_generation);
            }
            if let Some(mut diff) =
                world.get_resource_mut::<crate::semantic::viewport::SemanticDiffState>()
            {
                diff.reset_for_activation();
            }
            if let Some(mut catalogue) =
                world.get_resource_mut::<crate::bim::BimClassificationFieldCatalogueState>()
            {
                catalogue.clear();
            }
        },
        on_reset_source_derived: |world, activation_generation| {
            if let Some(mut semantic) =
                world.get_resource_mut::<crate::semantic::viewport::SemanticSyncState>()
            {
                semantic.reset_for_activation(activation_generation);
            }
            if let Some(mut diff) =
                world.get_resource_mut::<crate::semantic::viewport::SemanticDiffState>()
            {
                diff.reset_for_activation();
            }
            if let Some(mut catalogue) =
                world.get_resource_mut::<crate::bim::BimClassificationFieldCatalogueState>()
            {
                catalogue.clear();
            }
        },
        is_semantic_ready: |world| {
            world
                .get_resource::<crate::semantic::viewport::SemanticSyncState>()
                .is_some_and(|semantic| {
                    semantic.snapshot().is_some() && semantic.shared_bim_index().is_some()
                })
        },
        on_reset_document_edit_state: crate::api::reset_document_edit_state,
        compute_project_target_hash: crate::project::assets::closure::target_content_hash,
        revalidate_project_cache_identity: |project_root, expected| {
            let current = crate::cache::ProjectCacheIdentity::for_project(
                project_root,
                expected.target.clone(),
                expected.profile,
                expected.config_hash,
            )?;
            Ok(current == *expected)
        },
    }
}
