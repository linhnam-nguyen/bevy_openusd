use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use bevy_frost::prelude::*;
use bevy_frost::style;

use crate::viewport::diagnostics::log_capture::{LoaderLog, LogLine};
use crate::viewport::ui_frost::constants::{PANEL_H, PANEL_W, RIB_LOG, RIBBON_ITEMS, RIBBONS};
use crate::viewport::ui_frost::plugin::is_panel_open;

/// Displays the in-app log buffer with level filtering and target shortening.
pub fn draw_log_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    log: Res<LoaderLog>,
) {
    if !is_panel_open(&open, RIB_LOG) {
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
        RIB_LOG,
        "Log",
        egui::vec2(PANEL_W + 80.0, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("log_lines", "Loader log", true, |ui| {
                let count = log.buffer.lock().map(|b| b.len()).unwrap_or(0);
                sub_caption(ui, &format!("{count} entries · capped at 500"));
                ui.horizontal(|ui| {
                    if ui.small_button("Clear").clicked()
                        && let Ok(mut buf) = log.buffer.lock()
                    {
                        buf.clear();
                    }
                });
                ui.add_space(style::space::TIGHT);

                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        let snapshot: Vec<LogLine> = log
                            .buffer
                            .lock()
                            .map(|b| b.iter().cloned().collect())
                            .unwrap_or_default();
                        if snapshot.is_empty() {
                            sub_caption(ui, "(no events yet — load a stage)");
                            return;
                        }
                        for line in &snapshot {
                            let level_color = level_to_color(line.level);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                ui.painter().rect_filled(
                                    egui::Rect::from_center_size(
                                        ui.cursor().min + egui::vec2(4.0, 8.0),
                                        egui::vec2(6.0, 6.0),
                                    ),
                                    egui::CornerRadius::same(1),
                                    level_color,
                                );
                                ui.add_space(10.0);
                                ui.label(
                                    egui::RichText::new(short_target(&line.target))
                                        .small()
                                        .monospace()
                                        .color(style::TEXT_SECONDARY),
                                );
                                ui.label(
                                    egui::RichText::new(&line.message)
                                        .small()
                                        .color(style::TEXT_PRIMARY),
                                );
                            });
                        }
                    });
            });
        },
    );
}

/// Maps a Bevy log severity to the panel's readable foreground colour.
pub(crate) fn level_to_color(level: bevy::log::Level) -> egui::Color32 {
    match level {
        bevy::log::Level::ERROR => style::DANGER,
        bevy::log::Level::WARN => style::WARNING,
        bevy::log::Level::INFO => style::SUCCESS,
        _ => style::TEXT_SECONDARY,
    }
}

/// Condenses a Rust module path for narrow log-panel rows.
pub(crate) fn short_target(target: &str) -> String {
    target.rsplit("::").next().unwrap_or(target).to_string()
}
