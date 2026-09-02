use super::super::helpers::{emit_editor_completed, emit_snapshot, reject};
use super::state::ApplyViewportCommandState;
use crate::viewport::api::ViewportEventOutbox;
use viewport_protocol::EditorOperation;

pub(super) fn set_variant_selection(
    request_id: String,
    prim_path: String,
    set_name: String,
    option: String,
    outbox: &mut ViewportEventOutbox,
    state: &mut ApplyViewportCommandState<'_, '_>,
) {
    state
        .tuning
        .variants
        .insert((prim_path.clone(), set_name.clone()), option.clone());
    if let Some(stage) = state.stage.as_deref() {
        if stage.is_authoring_frozen() {
            reject(
                outbox,
                request_id,
                "LiveStage authoring is leased by Project publication".to_owned(),
            );
            return;
        }
        let suppression = stage.mark_authored_guard(prim_path.clone());
        if let Err(error) =
            state
                .histories
                .authoring
                .set_variant(&stage.stage, &prim_path, &set_name, &option)
        {
            reject(outbox, request_id, error.to_string());
            return;
        }
        suppression.commit();
        state
            .histories
            .record(super::super::state::EditorHistoryDomain::Authoring);
    }
    emit_snapshot(
        outbox,
        request_id.clone(),
        &state.configuration.p0(),
        &state.spawned,
        &state.selected_targets.0,
        state.selected_targets.revision(),
        &state.viewer_settings.0,
        &state.scene_index,
        &state.camera_mount,
        &state.camera_orientation.latest,
        &state.clock,
        &state.toggles,
        &state.tuning,
        state.physics.0,
    );
    emit_editor_completed(
        outbox,
        request_id,
        EditorOperation::SetVariantSelection,
        vec![format!("{prim_path}.{set_name}")],
        &state.histories,
    );
}
