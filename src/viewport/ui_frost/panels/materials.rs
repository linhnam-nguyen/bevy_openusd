use bevy::asset::Assets;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use bevy_frost::prelude::*;
use bevy_frost::style;
use usd_bevy::UsdPrimRef;

use crate::viewport::ui_frost::constants::{
    PANEL_H, PANEL_W, RIB_MATERIALS, RIBBON_ITEMS, RIBBONS,
};
use crate::viewport::ui_frost::plugin::is_panel_open;

/// Shows material bindings and material properties for the current stage.
pub fn draw_materials_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    usd_mesh_mats: Query<&MeshMaterial3d<StandardMaterial>, With<UsdPrimRef>>,
) {
    if !is_panel_open(&open, RIB_MATERIALS) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col = accent.0;
    let mut keep = true;

    let mut bound: std::collections::HashSet<AssetId<StandardMaterial>> =
        std::collections::HashSet::new();
    for mm in usd_mesh_mats.iter() {
        bound.insert(mm.0.id());
    }

    let mut entries: Vec<(AssetId<StandardMaterial>, String)> = materials
        .iter()
        .filter(|(id, _)| bound.contains(id))
        .map(|(id, _)| {
            let label = asset_server
                .get_path(id)
                .map(|p| p.to_string())
                .unwrap_or_else(|| format!("{id:?}"));
            (id, label)
        })
        .collect();
    entries.sort_by(|a, b| a.1.cmp(&b.1));

    floating_window_for_item(
        ctx,
        RIBBONS,
        RIBBON_ITEMS,
        &placement,
        RIB_MATERIALS,
        "Materials",
        egui::vec2(PANEL_W, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section(
                "materials_overview",
                &format!("{} material(s)", entries.len()),
                true,
                |ui| {
                    sub_caption(ui, "Edits update every mesh bound to that material.");
                },
            );
            for (id, label) in &entries {
                let short = label
                    .rsplit('/')
                    .next()
                    .unwrap_or(label)
                    .chars()
                    .take(48)
                    .collect::<String>();
                let section_id = format!("mat_{:?}", id);
                pane.section(
                    Box::leak(section_id.into_boxed_str()),
                    Box::leak(short.into_boxed_str()),
                    false,
                    |ui| {
                        let Some(mut mat) = materials.get_mut(*id) else {
                            return;
                        };
                        ui.label(egui::RichText::new(label).small().monospace());
                        ui.add_space(style::space::BLOCK);
                        let linear = mat.base_color.to_linear();
                        let mut rgb = [linear.red, linear.green, linear.blue];
                        ui.horizontal(|ui| {
                            ui.label("Base color:");
                            if ui.color_edit_button_rgb(&mut rgb).changed() {
                                mat.base_color = Color::LinearRgba(LinearRgba {
                                    red: rgb[0],
                                    green: rgb[1],
                                    blue: rgb[2],
                                    alpha: linear.alpha,
                                });
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Roughness:");
                            ui.add(
                                egui::Slider::new(&mut mat.perceptual_roughness, 0.0..=1.0)
                                    .step_by(0.01),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("Metallic:");
                            ui.add(egui::Slider::new(&mut mat.metallic, 0.0..=1.0).step_by(0.01));
                        });
                    },
                );
            }
        },
    );
}
