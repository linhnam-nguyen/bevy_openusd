use bevy::prelude::*;
use bevy_egui::EguiContexts;
use bevy_frost::prelude::*;
use viewport_protocol::ViewportCommand;

use crate::viewport::api::{ViewportCommandInbox, ViewportReadModelState};
use crate::viewport::session::LoadRequest;
use crate::viewport::ui_frost::constants::{
    PALETTE_ITEMS, RIB_CAMERAS, RIB_INFO, RIB_KEYS, RIB_LOG, RIB_OVERLAYS, RIB_SELECTION,
    RIB_TIMELINE, RIB_TREE, RIB_VARIANTS, RIBBON_LEFT, ViewerCommandPalette,
};

#[allow(clippy::too_many_arguments)]
/// Draws the command palette and dispatches the selected action.
pub fn draw_palette_panel(
    mut contexts: EguiContexts,
    accent: Res<AccentColor>,
    mut palette: ResMut<ViewerCommandPalette>,
    mut ribbon: ResMut<RibbonOpen>,
    read_model: Res<ViewportReadModelState>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
    mut load_req: ResMut<LoadRequest>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let Some(id) = command_palette(ctx, &mut palette.0, PALETTE_ITEMS, accent.0) else {
        return;
    };
    match id {
        "open_selection" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_SELECTION);
        }
        "open_tree" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_TREE);
        }
        "open_info" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_INFO);
        }
        "open_variants" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_VARIANTS);
        }
        "open_cameras" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_CAMERAS);
        }
        "open_overlays" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_OVERLAYS);
        }
        "open_timeline" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_TIMELINE);
        }
        "open_keys" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_KEYS);
        }
        "open_log" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_LOG);
        }
        "toggle_grid" => {
            if let Some(snapshot) = read_model.snapshot() {
                viewport_commands.send(ViewportCommand::SetOverlay {
                    overlay: viewport_protocol::OverlayKind::GroundGrid,
                    enabled: !snapshot.presentation.ground_grid,
                });
            }
        }
        "toggle_axes" => {
            if let Some(snapshot) = read_model.snapshot() {
                viewport_commands.send(ViewportCommand::SetOverlay {
                    overlay: viewport_protocol::OverlayKind::WorldAxes,
                    enabled: !snapshot.presentation.world_axes,
                });
            }
        }
        "toggle_markers" => {
            if let Some(snapshot) = read_model.snapshot() {
                viewport_commands.send(ViewportCommand::SetOverlay {
                    overlay: viewport_protocol::OverlayKind::PrimMarkers,
                    enabled: !snapshot.presentation.prim_markers,
                });
            }
        }
        "toggle_wireframe" => {
            if let Some(snapshot) = read_model.snapshot() {
                viewport_commands.send(ViewportCommand::SetOverlay {
                    overlay: viewport_protocol::OverlayKind::Wireframe,
                    enabled: !snapshot.presentation.wireframe,
                });
            }
        }
        "reload_stage" => {
            viewport_commands.send(ViewportCommand::ReloadSession);
        }
        "browse_usd" => {
            if let Some(picked) = rfd::FileDialog::new()
                .add_filter("USD stages", &["usda", "usdc", "usd", "usdz"])
                .pick_file()
            {
                load_req.path = Some(picked);
            }
        }
        _ => {}
    }
    palette.0.open = false;
}
