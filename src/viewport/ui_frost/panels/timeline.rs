use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use bevy_frost::prelude::*;
use bevy_frost::style;
use viewport_protocol::ViewportCommand;

use crate::viewport::api::{ViewportCommandInbox, ViewportReadModelState};
use crate::viewport::session::StageInfo;
use crate::viewport::ui_frost::constants::{
    PANEL_H, PANEL_W, RIB_KEYS, RIB_TIMELINE, RIBBON_ITEMS, RIBBONS,
};
use crate::viewport::ui_frost::plugin::is_panel_open;

/// Draws playback, scrub, and animation-clip controls for USD time samples.
pub fn draw_timeline_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    read_model: Res<ViewportReadModelState>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
    stage_info: Res<StageInfo>,
) {
    if !is_panel_open(&open, RIB_TIMELINE) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let Some(snapshot) = read_model.snapshot() else {
        return;
    };
    let timeline = &snapshot.timeline;
    let duration_seconds = (timeline.end_time_code - timeline.start_time_code).max(0.0)
        / timeline.time_codes_per_second.max(f64::MIN_POSITIVE);
    let accent_col = accent.0;
    let mut keep = true;
    floating_window_for_item(
        ctx,
        RIBBONS,
        RIBBON_ITEMS,
        &placement,
        RIB_TIMELINE,
        "Timeline",
        egui::vec2(PANEL_W, 320.0),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("timeline_playback", "Playback", true, |ui| {
                let animated_count = stage_info.animated_prim_count;
                sub_caption(
                    ui,
                    &format!(
                        "{animated_count} animated prim(s) · {:.1} fps · {:.1}s total",
                        timeline.time_codes_per_second, duration_seconds
                    ),
                );
                ui.add_space(style::space::BLOCK);

                let play_label = if timeline.playing {
                    "⏸  Pause"
                } else {
                    "▶  Play"
                };
                if wide_button(ui, play_label, accent_col).clicked() {
                    viewport_commands.send(ViewportCommand::SetPlayback {
                        playing: !timeline.playing,
                    });
                }
                if wide_button(ui, "⏮  Rewind", accent_col).clicked() {
                    viewport_commands.send(ViewportCommand::Seek { seconds: 0.0 });
                }

                ui.add_space(style::space::BLOCK);
                let dur = duration_seconds.max(1e-3);
                let mut seconds = timeline.seconds;
                if pretty_slider(ui, "Seconds", &mut seconds, 0.0..=dur, 3, " s", accent_col)
                    .changed()
                {
                    viewport_commands.send(ViewportCommand::Seek { seconds });
                }

                readout_row(
                    ui,
                    "timeCode",
                    &format!(
                        "{:.3}",
                        timeline.start_time_code
                            + timeline.seconds * timeline.time_codes_per_second
                    ),
                );
                readout_row(
                    ui,
                    "range",
                    &format!(
                        "{:.2} … {:.2}",
                        timeline.start_time_code, timeline.end_time_code
                    ),
                );
                readout_row(ui, "fps", &format!("{:.2}", timeline.time_codes_per_second));
            });
        },
    );
}

/// Displays the viewer's keyboard and mouse interaction reference.
pub fn draw_keys_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
) {
    if !is_panel_open(&open, RIB_KEYS) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col = accent.0;
    let mut keep = true;
    floating_window_for_item(
        ctx,
        RIBBONS,
        RIBBON_ITEMS,
        &placement,
        RIB_KEYS,
        "Controls",
        egui::vec2(PANEL_W, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("keys_camera", "Camera", true, |ui| {
                keybinding_row(ui, "L+R drag", "Orbit");
                keybinding_row(ui, "Middle", "Pan");
                keybinding_row(ui, "Scroll", "Zoom");
            });
            pane.section("keys_panels", "Panels", true, |ui| {
                keybinding_row(ui, "T", "Toggle prim tree");
                keybinding_row(ui, "I", "Toggle stage info");
                keybinding_row(ui, "O", "Toggle overlays");
                keybinding_row(ui, "?", "Toggle this panel");
            });
            pane.section("keys_overlays", "Overlays", true, |ui| {
                keybinding_row(ui, "G", "Ground grid");
                keybinding_row(ui, "X", "World axes");
                keybinding_row(ui, "P", "Prim markers");
                keybinding_row(ui, "B", "Skeleton bones");
            });
            pane.section("keys_stage", "Stage", true, |ui| {
                keybinding_row(ui, "R", "Reload stage from disk");
            });
        },
    );
    let _ = accent_col;
}
