use super::super::state::EditorHistories;
use super::read_model::{build_read_model, presentation_read_model};
use crate::viewport::animation::UsdStageTime;
use crate::viewport::api::ViewportEventOutbox;
use crate::viewport::bim::BimClassificationFieldCatalogueState;
use crate::viewport::camera::CameraMount;
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::scene::{SceneAnchorIndex, SelectedTargets};
use crate::viewport::stage::{LoaderTuning, Spawned, StageInfo};
use viewport_protocol::{
    CameraOrientationReadModel, EditorOperation, RuntimeMutationBatch, SelectionReadModel,
    ViewerSettingsReadModel, ViewportEvent, ViewportEventEnvelope,
};

/// Publishes the authoritative net selection change without repeating the
/// complete selected-anchor set on every incremental event. Full selection
/// state remains available in snapshots.
pub(in crate::viewport::api::bridge) fn emit_selection_delta(
    request_id: String,
    selection: &SelectedTargets,
    outbox: &mut ViewportEventOutbox,
) {
    let delta = selection.last_transaction_delta();
    let mut added = delta.added.iter().cloned().collect::<Vec<_>>();
    let mut removed = delta.removed.iter().cloned().collect::<Vec<_>>();
    added.sort_unstable();
    removed.sort_unstable();
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::SelectionDeltaApplied {
            revision: selection.revision(),
            added,
            removed,
            primary: selection.0.primary.clone(),
            count: selection.0.targets.len() as u32,
        },
    ));
}

#[allow(clippy::too_many_arguments)]
pub(in crate::viewport::api::bridge) fn emit_snapshot(
    outbox: &mut ViewportEventOutbox,
    request_id: String,
    stage_info: &StageInfo,
    spawned: &Spawned,
    selection: &SelectionReadModel,
    selection_revision: u64,
    settings: &ViewerSettingsReadModel,
    scene_index: &SceneAnchorIndex,
    camera_mount: &CameraMount,
    camera_orientation: &CameraOrientationReadModel,
    clock: &UsdStageTime,
    toggles: &DisplayToggles,
    tuning: &LoaderTuning,
    physics_running: bool,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::Snapshot {
            state: Box::new(build_read_model(
                stage_info,
                spawned.0,
                selection,
                selection_revision,
                settings,
                scene_index,
                camera_mount,
                camera_orientation,
                clock,
                toggles,
                tuning,
                physics_running,
            )),
        },
    ));
}

pub(in crate::viewport::api::bridge) fn emit_classification_field_catalogue(
    outbox: &mut ViewportEventOutbox,
    request_id: String,
    catalogue: Option<&BimClassificationFieldCatalogueState>,
) {
    let Some(catalogue) = catalogue else {
        return;
    };
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::BimClassificationFieldCatalogueChanged {
            catalogue: catalogue.current().clone(),
        },
    ));
}

pub(in crate::viewport::api::bridge) fn emit_presentation_changed(
    outbox: &mut ViewportEventOutbox,
    request_id: String,
    toggles: &DisplayToggles,
    tuning: &LoaderTuning,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::PresentationChanged {
            presentation: presentation_read_model(toggles, tuning),
        },
    ));
}

pub(in crate::viewport::api::bridge) fn emit_viewer_settings_changed(
    outbox: &mut ViewportEventOutbox,
    request_id: String,
    settings: &ViewerSettingsReadModel,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::ViewerSettingsChanged {
            settings: settings.clone(),
        },
    ));
}

pub(in crate::viewport::api::bridge) fn reject(
    outbox: &mut ViewportEventOutbox,
    request_id: String,
    reason: String,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id.clone()),
        ViewportEvent::CommandRejected { request_id, reason },
    ));
}

pub(in crate::viewport::api::bridge) fn emit_editor_completed(
    outbox: &mut ViewportEventOutbox,
    request_id: String,
    operation: EditorOperation,
    changed_paths: Vec<String>,
    histories: &EditorHistories,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::EditorCommandCompleted {
            operation,
            changed_paths,
            state: histories.state(),
        },
    ));
}

pub(in crate::viewport::api::bridge) fn emit_runtime_mutation_accepted(
    outbox: &mut ViewportEventOutbox,
    request_id: String,
    batch: &RuntimeMutationBatch,
    changed_paths: Vec<String>,
    histories: &EditorHistories,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::RuntimeMutationBatchAccepted {
            source_id: batch.source_id.clone(),
            sequence: batch.sequence,
            base_revision: batch.base_revision,
            applied_operations: batch.operations.len() as u32,
            changed_paths,
            state: histories.state(),
        },
    ));
}

pub(in crate::viewport::api::bridge) fn emit_editor_export(
    outbox: &mut ViewportEventOutbox,
    request_id: &str,
    content: &str,
) {
    // Leave room for the event and session envelopes under the transport's
    // 12 KiB application-message ceiling.
    const CHUNK_BYTES: usize = 8 * 1024;
    let mut chunks = Vec::new();
    let mut current = String::new();
    for character in content.chars() {
        if !current.is_empty() && current.len() + character.len_utf8() > CHUNK_BYTES {
            chunks.push(std::mem::take(&mut current));
        }
        current.push(character);
    }
    if !current.is_empty() || chunks.is_empty() {
        chunks.push(current);
    }

    let export_id = format!("export-{request_id}");
    let chunk_count = chunks.len() as u32;
    for (chunk_index, content) in chunks.into_iter().enumerate() {
        outbox.push(ViewportEventEnvelope::new(
            Some(request_id.to_owned()),
            ViewportEvent::EditorStageExportChunk {
                export_id: export_id.clone(),
                chunk_index: chunk_index as u32,
                chunk_count,
                content,
            },
        ));
    }
}
