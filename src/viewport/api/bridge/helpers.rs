use viewport_protocol::{
    CameraSource, CurveTuning as ProtocolCurveTuning, EditorOperation, OverlayKind,
    PresentationReadModel, RuntimeMutationBatch, SceneAnchor, SelectionReadModel, StageReadModel,
    TimelineReadModel, ViewportEvent, ViewportEventEnvelope, ViewportReadModel, PROTOCOL_VERSION,
};

use crate::viewport::api::{SceneAnchorIndex, ViewportEventOutbox};
use crate::viewport::animation::UsdStageTime;
use crate::viewport::camera::CameraMount;
use crate::viewport::scene::SelectedPrim;
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::session::{LoaderTuning, Spawned, StageInfo};
use super::state::EditorHistories;

pub(super) fn resolve_anchor(
    anchor: &SceneAnchor,
    scene_index: &SceneAnchorIndex,
) -> Result<bevy::ecs::entity::Entity, String> {
    scene_index.resolve(anchor).ok_or_else(|| {
        format!(
            "target {} is not present in the active scene",
            anchor.prim_path
        )
    })
}

pub(super) fn set_overlay(
    toggles: &mut DisplayToggles,
    overlay: OverlayKind,
    enabled: bool,
) {
    match overlay {
        OverlayKind::GroundGrid => toggles.show_world_grid = enabled,
        OverlayKind::WorldAxes => toggles.show_world_axes = enabled,
        OverlayKind::PrimMarkers => toggles.show_prim_markers = enabled,
        OverlayKind::Skeleton => toggles.show_skeleton = enabled,
        OverlayKind::Physics => toggles.show_physics = enabled,
        OverlayKind::Colliders => toggles.show_colliders = enabled,
        OverlayKind::Wireframe => toggles.wireframe = enabled,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_snapshot(
    outbox: &mut ViewportEventOutbox,
    request_id: String,
    stage_info: &StageInfo,
    spawned: &Spawned,
    selected: &SelectedPrim,
    scene_index: &SceneAnchorIndex,
    camera_mount: &CameraMount,
    clock: &UsdStageTime,
    toggles: &DisplayToggles,
    tuning: &LoaderTuning,
    physics_running: bool,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::Snapshot {
            state: build_read_model(
                stage_info,
                spawned.0,
                selected,
                scene_index,
                camera_mount,
                clock,
                toggles,
                tuning,
                physics_running,
            ),
        },
    ));
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_read_model(
    stage_info: &StageInfo,
    stage_loaded: bool,
    selected: &SelectedPrim,
    scene_index: &SceneAnchorIndex,
    camera_mount: &CameraMount,
    clock: &UsdStageTime,
    toggles: &DisplayToggles,
    tuning: &LoaderTuning,
    physics_running: bool,
) -> ViewportReadModel {
    ViewportReadModel {
        protocol_version: PROTOCOL_VERSION,
        stage: StageReadModel {
            display_name: stage_info.path.clone(),
            loaded: stage_loaded,
        },
        scene: scene_index.roots_read_model(),
        selection: SelectionReadModel {
            target: selected.0.and_then(|entity| scene_index.anchor_for(entity)),
        },
        camera_source: match camera_mount {
            CameraMount::Arcball => CameraSource::Arcball,
            CameraMount::Mounted { prim_path } => CameraSource::Authored {
                prim_path: prim_path.clone(),
            },
        },
        timeline: timeline_read_model(clock),
        presentation: presentation_read_model(toggles, tuning),
        physics_running,
    }
}

pub(super) fn timeline_read_model(clock: &UsdStageTime) -> TimelineReadModel {
    TimelineReadModel {
        seconds: clock.seconds,
        playing: clock.playing,
        start_time_code: clock.start_time_code,
        end_time_code: clock.end_time_code,
        time_codes_per_second: clock.time_codes_per_second,
    }
}

pub(super) fn presentation_read_model(
    toggles: &DisplayToggles,
    tuning: &LoaderTuning,
) -> PresentationReadModel {
    PresentationReadModel {
        ground_grid: toggles.show_world_grid,
        ground_grid_origin: toggles.ground_grid_origin,
        world_axes: toggles.show_world_axes,
        prim_markers: toggles.show_prim_markers,
        prim_marker_bias: toggles.prim_marker_bias,
        skeleton: toggles.show_skeleton,
        physics: toggles.show_physics,
        colliders: toggles.show_colliders,
        wireframe: toggles.wireframe,
        light_intensity_scale: toggles.light_intensity_scale,
        curve_tuning: ProtocolCurveTuning {
            default_radius: tuning.curves.default_radius,
            ring_segments: tuning.curves.ring_segments,
            point_scale: tuning.curves.point_scale,
        },
    }
}

pub(super) fn emit_presentation_changed(
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

pub(super) fn reject(outbox: &mut ViewportEventOutbox, request_id: String, reason: String) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id.clone()),
        ViewportEvent::CommandRejected { request_id, reason },
    ));
}

pub(super) fn emit_editor_completed(
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

pub(super) fn emit_runtime_mutation_accepted(
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

pub(super) fn emit_editor_export(
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
