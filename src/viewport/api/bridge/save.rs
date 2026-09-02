//! Authoritative local persistence for the working USD stage.

use std::path::Path;

use usd_bevy::LiveStage;
use viewport_protocol::EditorOperation;

use super::helpers::{emit_editor_completed, reject};
use super::state::EditorHistories;
use crate::viewport::api::ViewportEventOutbox;

pub(super) fn save_stage_as(
    request_id: String,
    outbox: &mut ViewportEventOutbox,
    histories: &mut EditorHistories,
    stage: Option<&LiveStage>,
    filename: &str,
) {
    let Some(stage) = stage else {
        reject(outbox, request_id, "stage is not loaded".to_owned());
        return;
    };
    persist(
        request_id,
        outbox,
        histories,
        stage,
        filename,
        EditorOperation::SaveStageAs,
    );
}

pub(super) fn save_current_stage(
    request_id: String,
    outbox: &mut ViewportEventOutbox,
    histories: &mut EditorHistories,
    stage: Option<&LiveStage>,
    path: Option<&Path>,
) {
    let Some(stage) = stage else {
        reject(outbox, request_id, "stage is not loaded".to_owned());
        return;
    };
    let Some(path) = path else {
        reject(
            outbox,
            request_id,
            "current stage has no local save path".to_owned(),
        );
        return;
    };
    let filename = path.to_string_lossy();
    persist(
        request_id,
        outbox,
        histories,
        stage,
        &filename,
        EditorOperation::SaveStage,
    );
}

fn persist(
    request_id: String,
    outbox: &mut ViewportEventOutbox,
    histories: &mut EditorHistories,
    stage: &LiveStage,
    filename: &str,
    operation: EditorOperation,
) {
    if let Err(error) = usd_bevy::authoring::save_stage_as(&stage.stage, filename) {
        reject(outbox, request_id, error.to_string());
        return;
    }
    histories.mark_saved();
    emit_editor_completed(outbox, request_id, operation, Vec::new(), histories);
}
