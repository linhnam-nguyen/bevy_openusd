use std::path::Path;

use bevy::prelude::*;
use usd_bevy::LiveStage;
use viewport_protocol::{PROTOCOL_VERSION, ViewportEvent, ViewportEventEnvelope};

use crate::project::ghost_cache::HistoricalGeometryCache;
use crate::project::recovery::{RecoveryRuntimeState, RecoverySettings};
use crate::viewport::api::{
    SceneAnchorIndex, ViewportCommandInbox, ViewportEventOutbox, ViewportReadModelState,
    ViewportTreeCommandInbox,
};
use crate::viewport::semantic::{
    SemanticDiffState, SemanticSyncState, SemanticWorkingStore, synchronize_live_stage,
};
use crate::viewport::session::StageInfo;
use super::commands::apply_viewport_commands;
use super::scene_query::{
    dispatch_scene_query_commands, publish_semantic_query_results, publish_stage_load_state,
};
use super::state::{
    EditorHistories, RuntimeMutationCoordinator, SemanticSearchRequests, ViewportBridgeSet,
};
use super::tree::apply_tree_commands;

/// Installs the in-process implementation of the public viewport contract.
pub(crate) struct ViewportBridgePlugin;

impl Plugin for ViewportBridgePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ViewportCommandInbox>()
            .init_resource::<ViewportTreeCommandInbox>()
            .init_resource::<ViewportEventOutbox>()
            .init_resource::<ViewportReadModelState>()
            .init_resource::<SceneAnchorIndex>()
            .init_resource::<SemanticWorkingStore>()
            .init_resource::<SemanticSyncState>()
            .init_resource::<SemanticDiffState>()
            .init_resource::<SemanticSearchRequests>()
            .init_resource::<EditorHistories>()
            .init_resource::<RuntimeMutationCoordinator>()
            .init_resource::<RecoverySettings>()
            .init_resource::<RecoveryRuntimeState>()
            .init_resource::<HistoricalGeometryCache>()
            .add_systems(Startup, emit_viewport_ready)
            .configure_sets(
                Update,
                (
                    ViewportBridgeSet::RefreshSceneIndex,
                    ViewportBridgeSet::ApplyCommands,
                    ViewportBridgeSet::ApplyTreeCommands,
                    ViewportBridgeSet::PublishStageLoadState,
                    ViewportBridgeSet::ReduceEvents,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                super::super::scene_index::refresh_scene_anchor_index
                    .in_set(ViewportBridgeSet::RefreshSceneIndex),
            )
            // LiveStagePlugin drains and reprojects in Update. PostUpdate is
            // the first schedule where the retained batch is guaranteed to
            // represent the completed current-frame revision.
            .add_systems(
                PostUpdate,
                (synchronize_live_stage, checkpoint_recovery).chain(),
            )
            .add_systems(
                Update,
                (
                    publish_semantic_query_results,
                    dispatch_scene_query_commands,
                    apply_viewport_commands,
                )
                    .chain()
                    .in_set(ViewportBridgeSet::ApplyCommands),
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
    }
}

/// Writes one scratch checkpoint after the authoritative change fan-out has
/// completed for the frame. The retained batch makes this coarse-grained: no
/// checkpoint is written on idle frames or for every animation tick.
pub(super) fn checkpoint_recovery(
    settings: Res<RecoverySettings>,
    mut runtime: ResMut<RecoveryRuntimeState>,
    pending: Res<usd_bevy::PendingStageChanges>,
    stage: Option<NonSend<LiveStage>>,
    stage_info: Res<StageInfo>,
    mut counters: Option<ResMut<crate::viewport::diagnostics::performance::RendererCounters>>,
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

    let store = match runtime.store_for(&settings, stage.session_id()) {
        Ok(store) => store,
        Err(error) => {
            bevy::log::error!("[recovery] cannot create checkpoint store: {error:#}");
            return;
        }
    };
    if let Err(error) = store.write_checkpoint(&stage, Path::new(&stage_info.path), None) {
        bevy::log::error!("[recovery] checkpoint failed: {error:#}");
    } else if let Some(ref mut c) = counters {
        c.recovery_checkpoints += 1;
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
