use viewport_protocol::ViewportCommand;

use super::selection;
use crate::viewport::api::{SceneAnchorIndex, ViewportEventOutbox};
use crate::viewport::scene::{SelectedPrim, SelectedTargets};

pub(super) fn apply_selection_command(
    command: ViewportCommand,
    request_id: String,
    outbox: &mut ViewportEventOutbox,
    selected_prim: &mut SelectedPrim,
    selected_targets: &mut SelectedTargets,
    scene_index: &SceneAnchorIndex,
) -> Option<(ViewportCommand, String)> {
    match command {
        ViewportCommand::SelectTarget { target } => {
            selection::select_target(
                request_id,
                target,
                outbox,
                selected_prim,
                selected_targets,
                scene_index,
            );
            None
        }
        ViewportCommand::ReplaceSelection { targets, primary } => {
            selection::replace_selection(
                request_id,
                targets,
                primary,
                outbox,
                selected_prim,
                selected_targets,
                scene_index,
            );
            None
        }
        ViewportCommand::AddSelectionTarget {
            target,
            make_primary,
        } => {
            selection::add_selection_target(
                request_id,
                target,
                make_primary,
                outbox,
                selected_prim,
                selected_targets,
                scene_index,
            );
            None
        }
        ViewportCommand::AddSelectionTargets { targets, primary } => {
            selection::add_selection_targets(
                request_id,
                targets,
                primary,
                outbox,
                selected_prim,
                selected_targets,
                scene_index,
            );
            None
        }
        ViewportCommand::RemoveSelectionTarget { target } => {
            selection::remove_selection_target(
                request_id,
                target,
                outbox,
                selected_prim,
                selected_targets,
                scene_index,
            );
            None
        }
        ViewportCommand::RemoveSelectionTargets { targets } => {
            selection::remove_selection_targets(
                request_id,
                targets,
                outbox,
                selected_prim,
                selected_targets,
                scene_index,
            );
            None
        }
        ViewportCommand::ClearSelection => {
            selection::clear_selection(request_id, outbox, selected_prim, selected_targets);
            None
        }
        command => Some((command, request_id)),
    }
}
