use usd_bevy::LiveStage;
use usd_model::SemanticSnapshot;
use viewport_protocol::ViewportCommand;

use super::bim_edit;
use super::helpers::reject;
use super::state::EditorHistories;
use crate::viewport::api::ViewportEventOutbox;
use crate::viewport::scene::SelectedTargets;

/// Applies BIM commands before the general editor dispatcher consumes the
/// command. The error branch returns non-BIM commands without cloning them.
pub(super) fn try_apply_bim_command(
    command: ViewportCommand,
    request_id: String,
    outbox: &mut ViewportEventOutbox,
    histories: &mut EditorHistories,
    semantic_snapshot: Option<&SemanticSnapshot>,
    stage: Option<&LiveStage>,
    selected_targets: &SelectedTargets,
) -> Result<bool, (ViewportCommand, String)> {
    if !matches!(
        &command,
        ViewportCommand::EditBimProperty { .. }
            | ViewportCommand::EditBimProperties { .. }
            | ViewportCommand::ApplyBimReplacementBatch { .. }
    ) {
        return Err((command, request_id));
    }

    let Some(stage) = stage else {
        reject(outbox, request_id, "stage is not loaded".to_owned());
        return Ok(true);
    };

    match command {
        ViewportCommand::EditBimProperty { mutation } => {
            let outcome = bim_edit::apply_bim_property_mutation(
                stage,
                histories,
                semantic_snapshot,
                &mutation,
            );
            bim_edit::emit_bim_property_completed(
                outbox,
                request_id,
                outcome,
                stage.current_revision().0,
                histories,
            );
        }
        ViewportCommand::EditBimProperties {
            selection_revision,
            mutations,
        } => {
            bim_edit::apply_bim_property_batch_command(
                stage,
                histories,
                semantic_snapshot,
                selection_revision,
                selected_targets,
                mutations,
                outbox,
                request_id,
            );
        }
        ViewportCommand::ApplyBimReplacementBatch { mutations } => {
            bim_edit::apply_bim_replacement_batch_command(
                stage,
                histories,
                semantic_snapshot,
                mutations,
                outbox,
                request_id,
            );
        }
        _ => unreachable!("BIM command was checked before dispatch"),
    }
    Ok(true)
}
