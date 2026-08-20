use bevy::mesh::Mesh3d;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use bevy_frost::prelude::*;
use usd_bevy::{UsdPrimRef, UsdProcedural, UsdSpatialAudio};
use viewport_protocol::ViewportCommand;

use crate::viewport::api::ViewportCommandInbox;
use crate::viewport::semantic::SemanticDiffState;
use crate::viewport::session::StageInfo;
use crate::viewport::ui_frost::constants::{PANEL_H, PANEL_W, RIB_INFO, RIBBON_ITEMS, RIBBONS};
use crate::viewport::ui_frost::plugin::is_panel_open;

#[allow(clippy::too_many_arguments)]
/// Draws the loaded-stage metadata and projection summary panel.
pub fn draw_info_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    info: Res<StageInfo>,
    mut diff: ResMut<SemanticDiffState>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
    prims: Query<&UsdPrimRef>,
    meshes_q: Query<&Mesh3d, With<UsdPrimRef>>,
    spatial_audio_q: Query<&UsdSpatialAudio>,
    procedural_q: Query<&UsdProcedural>,
) {
    if !is_panel_open(&open, RIB_INFO) {
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
        RIB_INFO,
        "Stage info",
        egui::vec2(PANEL_W, PANEL_H + 40.0),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("info_stage", "Stage", true, |ui| {
                readout_row(ui, "file", &info.path);
                readout_row(
                    ui,
                    "defaultPrim",
                    info.default_prim.as_deref().unwrap_or("—"),
                );
                readout_row(ui, "layers", &info.layer_count.to_string());
                readout_row(ui, "prims", &prims.iter().count().to_string());
                readout_row(ui, "meshes", &meshes_q.iter().count().to_string());
                readout_row(ui, "variants", &info.variant_count.to_string());
            });
            pane.section("info_diff", "Working diff", true, |ui| {
                if !diff.has_baseline() {
                    if diff.has_working_snapshot() {
                        sub_caption(ui, "No manual baseline captured");
                        if wide_button(ui, "Capture current as baseline", accent_col).clicked() {
                            diff.capture_baseline();
                        }
                    } else {
                        sub_caption(ui, "Waiting for semantic snapshot");
                    }
                } else if let Some(summary) = diff.summary() {
                    readout_row(ui, "added", &summary.added.to_string());
                    readout_row(ui, "removed", &summary.removed.to_string());
                    readout_row(ui, "changed", &summary.changed.to_string());
                    readout_row(ui, "unchanged", &summary.unchanged.to_string());
                    let flag_labels = [
                        format!("{} transform", summary.transform),
                        format!("{} geometry", summary.geometry),
                        format!("{} metadata", summary.metadata),
                        format!("{} path", summary.path),
                    ];
                    let refs: Vec<&str> = flag_labels.iter().map(String::as_str).collect();
                    badge_row(ui, "flags", &refs, accent_col);
                    if wide_button(ui, "Clear manual baseline", accent_col).clicked() {
                        diff.clear_baseline();
                    }
                }
            });
            pane.section("info_lights", "Lights & instances", true, |ui| {
                let light_labels = [
                    format!("{} dir", info.lights_directional),
                    format!("{} pt", info.lights_point),
                    format!("{} spot", info.lights_spot),
                    format!("{} dome", info.lights_dome),
                ];
                let refs: Vec<&str> = light_labels.iter().map(String::as_str).collect();
                badge_row(ui, "lights", &refs, accent_col);

                let inst_labels = [
                    format!("{} prim", info.instance_prim_count),
                    format!("{} reuse", info.instance_prototype_reuses),
                ];
                let refs: Vec<&str> = inst_labels.iter().map(String::as_str).collect();
                badge_row(ui, "instances", &refs, accent_col);

                readout_row(
                    ui,
                    "animated",
                    &format!("{} prim(s)", info.animated_prim_count),
                );
            });
            pane.section("info_skel_render", "Skel & render", true, |ui| {
                let skel_labels = [
                    format!("{} skel", info.skeleton_count),
                    format!("{} root", info.skel_root_count),
                    format!("{} bind", info.skel_binding_count),
                ];
                let refs: Vec<&str> = skel_labels.iter().map(String::as_str).collect();
                badge_row(ui, "skel", &refs, accent_col);

                let render_labels = [
                    format!("{} settings", info.render_settings_count),
                    format!("{} product", info.render_product_count),
                    format!("{} var", info.render_var_count),
                ];
                let refs: Vec<&str> = render_labels.iter().map(String::as_str).collect();
                badge_row(ui, "render", &refs, accent_col);

                if let Some([w, h]) = info.render_primary_resolution {
                    readout_row(ui, "resolution", &format!("{w} × {h}"));
                }

                let phys_labels = [
                    format!("{} scene", info.physics_scene_count),
                    format!("{} rigid", info.rigid_body_count),
                    format!("{} joint", info.joint_count),
                ];
                let refs: Vec<&str> = phys_labels.iter().map(String::as_str).collect();
                badge_row(ui, "physics", &refs, accent_col);
            });
            pane.section("info_authoring", "Authoring detail", true, |ui| {
                readout_row(
                    ui,
                    "custom",
                    &format!(
                        "{} prim · {} layer entries",
                        info.custom_attr_prim_count, info.custom_layer_data_entries
                    ),
                );
                readout_row(
                    ui,
                    "subdiv",
                    &format!("{} mesh(es) subdivision", info.subdivision_prim_count),
                );
                readout_row(
                    ui,
                    "light-link",
                    &format!("{} light(s) linked", info.light_linked_count),
                );
                readout_row(
                    ui,
                    "clips",
                    &format!("{} prim(s) UsdClipsAPI", info.clip_prim_count),
                );
                readout_row(
                    ui,
                    "spatial-audio",
                    &format!("{} source(s)", spatial_audio_q.iter().count()),
                );
                readout_row(
                    ui,
                    "procedural",
                    &format!("{} prim(s)", procedural_q.iter().count()),
                );
            });
            pane.section("info_actions", "Actions", true, |ui| {
                if wide_button(ui, "⟳  Reload stage (R)", accent_col).clicked() {
                    viewport_commands.send(ViewportCommand::ReloadSession);
                }
            });
        },
    );
}
