use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use bevy_frost::prelude::*;
use viewport_protocol::ViewportCommand;

use crate::viewport::api::{ViewportCommandInbox, ViewportReadModelState};
use crate::viewport::session::LoadRequest;
use crate::viewport::ui_frost::constants::{
    PANEL_H, PANEL_W, RIB_SELECTION, RIBBON_ITEMS, RIBBONS,
};
use crate::viewport::ui_frost::plugin::is_panel_open;

/// Draws the stage picker and details for the authoritative selected target.
pub fn draw_selection_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    requested: Res<crate::viewport::session::RequestedAsset>,
    mut load_req: ResMut<LoadRequest>,
    read_model: Res<ViewportReadModelState>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
) {
    if !is_panel_open(&open, RIB_SELECTION) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let stage_name = read_model
        .snapshot()
        .map(|snapshot| snapshot.stage.display_name.clone())
        .unwrap_or_else(|| "(stage loading)".to_owned());
    let selection = read_model
        .snapshot()
        .and_then(|snapshot| snapshot.selection.target.clone());
    let accent_col = accent.0;
    let mut keep = true;
    floating_window_for_item(
        ctx,
        RIBBONS,
        RIBBON_ITEMS,
        &placement,
        RIB_SELECTION,
        "Selection",
        egui::vec2(PANEL_W, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("sel_stage", "Loaded stage", true, |ui| {
                readout_row(ui, "file", &stage_name);
                if wide_button(ui, "📁  Browse USD…", accent_col).clicked()
                    && let Some(picked) = rfd::FileDialog::new()
                        .add_filter("USD stages", &["usda", "usdc", "usd", "usdz"])
                        .pick_file()
                {
                    load_req.path = Some(picked);
                }
                if wide_button(ui, "🗂  Reveal in filesystem", accent_col).clicked() {
                    let full = requested.root.join(&stage_name);
                    let target = full.parent().unwrap_or(&requested.root).to_path_buf();
                    let _ = std::process::Command::new("xdg-open").arg(&target).spawn();
                }
            });
            pane.section("sel_prim", "Selected prim", true, |ui| match &selection {
                Some(target) => {
                    readout_row(
                        ui,
                        "name",
                        target
                            .prim_path
                            .rsplit('/')
                            .next()
                            .unwrap_or(&target.prim_path),
                    );
                    readout_row(ui, "path", &target.prim_path);
                    if let Some(context) = &target.instance_context {
                        readout_row(ui, "instance", context);
                    }
                    if wide_button(ui, "Clear selection", accent_col).clicked() {
                        viewport_commands.send(ViewportCommand::SelectTarget { target: None });
                    }
                }
                None => sub_caption(ui, "Click a prim in the Tree panel"),
            });
        },
    );
}
