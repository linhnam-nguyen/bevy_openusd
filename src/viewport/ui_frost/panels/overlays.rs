use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use bevy_frost::prelude::*;
use viewport_protocol::ViewportCommand;

use crate::viewport::api::{ViewportCommandInbox, ViewportReadModelState};
use crate::viewport::ui_frost::constants::{PANEL_H, PANEL_W, RIB_OVERLAYS, RIBBON_ITEMS, RIBBONS};
use crate::viewport::ui_frost::plugin::is_panel_open;

/// Sends a Frost toggle through the same presentation command used by a host UI.
pub fn protocol_overlay_toggle(
    ui: &mut egui::Ui,
    label: &str,
    current: bool,
    overlay: viewport_protocol::OverlayKind,
    accent: egui::Color32,
    commands: &mut ViewportCommandInbox,
) {
    let mut value = current;
    if toggle(ui, label, &mut value, accent).changed() {
        commands.send(ViewportCommand::SetOverlay {
            overlay,
            enabled: value,
        });
    }
}

/// Exposes debug-overlay, wireframe, lighting, and collider visibility controls.
pub fn draw_overlays_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    read_model: Res<ViewportReadModelState>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
) {
    if !is_panel_open(&open, RIB_OVERLAYS) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let Some(snapshot) = read_model.snapshot() else {
        return;
    };
    let presentation = &snapshot.presentation;
    let accent_col = accent.0;
    let mut keep = true;
    let mut curve_tuning = presentation.curve_tuning;
    floating_window_for_item(
        ctx,
        RIBBONS,
        RIBBON_ITEMS,
        &placement,
        RIB_OVERLAYS,
        "Overlays",
        egui::vec2(PANEL_W, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("overlay_toggles", "World overlays", true, |ui| {
                protocol_overlay_toggle(
                    ui,
                    "Ground grid (G)",
                    presentation.ground_grid,
                    viewport_protocol::OverlayKind::GroundGrid,
                    accent_col,
                    &mut viewport_commands,
                );
                protocol_overlay_toggle(
                    ui,
                    "World axes (X)",
                    presentation.world_axes,
                    viewport_protocol::OverlayKind::WorldAxes,
                    accent_col,
                    &mut viewport_commands,
                );
                protocol_overlay_toggle(
                    ui,
                    "Prim markers (P)",
                    presentation.prim_markers,
                    viewport_protocol::OverlayKind::PrimMarkers,
                    accent_col,
                    &mut viewport_commands,
                );
                let mut v = presentation.prim_marker_bias as f64;
                if pretty_slider(
                    ui,
                    "Prim marker bias",
                    &mut v,
                    0.0..=5.0,
                    2,
                    "×",
                    accent_col,
                )
                .changed()
                {
                    viewport_commands.send(ViewportCommand::SetPrimMarkerBias { bias: v as f32 });
                }
                protocol_overlay_toggle(
                    ui,
                    "Skeleton bones (B)",
                    presentation.skeleton,
                    viewport_protocol::OverlayKind::Skeleton,
                    accent_col,
                    &mut viewport_commands,
                );
                protocol_overlay_toggle(
                    ui,
                    "Physics gizmos (Y)",
                    presentation.physics,
                    viewport_protocol::OverlayKind::Physics,
                    accent_col,
                    &mut viewport_commands,
                );
                protocol_overlay_toggle(
                    ui,
                    "Collider wireframes (C)",
                    presentation.colliders,
                    viewport_protocol::OverlayKind::Colliders,
                    accent_col,
                    &mut viewport_commands,
                );
            });

            pane.section("overlay_render", "Render", true, |ui| {
                protocol_overlay_toggle(
                    ui,
                    "Wireframe",
                    presentation.wireframe,
                    viewport_protocol::OverlayKind::Wireframe,
                    accent_col,
                    &mut viewport_commands,
                );
                let mut s = presentation.light_intensity_scale as f64;
                if pretty_slider(ui, "Light intensity", &mut s, 0.0..=5.0, 2, "×", accent_col)
                    .changed()
                {
                    viewport_commands.send(ViewportCommand::SetLightIntensity { scale: s as f32 });
                }
                sub_caption(ui, "Scales every authored light from its original value.");
            });

            pane.section("overlay_curves", "Curves (tubes)", true, |ui| {
                sub_caption(ui, "Default radius used when widths aren't authored");
                let mut r = curve_tuning.default_radius as f64;
                if pretty_slider(ui, "Radius", &mut r, 0.001..=0.2, 3, " m", accent_col).changed() {
                    curve_tuning.default_radius = r as f32;
                    viewport_commands.send(ViewportCommand::SetCurveTuning {
                        tuning: curve_tuning,
                    });
                }
                let mut seg = curve_tuning.ring_segments as f64;
                if pretty_slider(ui, "Ring segments", &mut seg, 3.0..=24.0, 0, "", accent_col)
                    .changed()
                {
                    curve_tuning.ring_segments = seg.round() as u32;
                    viewport_commands.send(ViewportCommand::SetCurveTuning {
                        tuning: curve_tuning,
                    });
                }
                let mut ps = curve_tuning.point_scale as f64;
                if pretty_slider(ui, "Point scale", &mut ps, 0.05..=4.0, 2, "×", accent_col)
                    .changed()
                {
                    curve_tuning.point_scale = ps as f32;
                    viewport_commands.send(ViewportCommand::SetCurveTuning {
                        tuning: curve_tuning,
                    });
                }
                sub_caption(ui, "Sliders apply live — no reload needed.");
            });
        },
    );
}
