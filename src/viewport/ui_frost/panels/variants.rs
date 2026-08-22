use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use bevy_frost::prelude::*;
use bevy_frost::style;
use bevy_frost::widgets::section as nested_section;
use std::hash::Hash;
use viewport_protocol::ViewportCommand;

use crate::viewport::api::ViewportCommandInbox;
use crate::viewport::session::{LoaderTuning, StageInfo};
use crate::viewport::ui_frost::constants::{PANEL_H, PANEL_W, RIB_VARIANTS, RIBBON_ITEMS, RIBBONS};
use crate::viewport::ui_frost::plugin::is_panel_open;

/// Draws variant-set controls and records pending reload selections.
pub fn draw_variants_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    stage_info: Res<StageInfo>,
    loader_tuning: Res<LoaderTuning>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
) {
    if !is_panel_open(&open, RIB_VARIANTS) {
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
        RIB_VARIANTS,
        "Variants",
        egui::vec2(PANEL_W, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("variants_animation", "Animation clips", true, |ui| {
                let asset = Some(&*stage_info);
                let Some(asset) = asset else {
                    sub_caption(ui, "(no stage loaded yet)");
                    return;
                };

                let mut anim_sets: Vec<_> = asset
                    .variants
                    .iter()
                    .flat_map(|(prim_path, sets)| {
                        sets.iter()
                            .filter(|set| set.name == "anim" && !set.options.is_empty())
                            .map(move |set| (prim_path, set))
                    })
                    .collect();
                anim_sets.sort_by(|a, b| a.0.cmp(b.0));

                if anim_sets.is_empty() {
                    if asset.skel_animation_count == 0 {
                        sub_caption(ui, "No UsdSkel animations or `anim` variant set found.");
                    } else {
                        sub_caption(
                            ui,
                            &format!(
                                "{} SkelAnimation prim(s) found; this stage does not expose an `anim` variant switch.",
                                asset.skel_animation_count
                            ),
                        );
                    }
                    return;
                }

                sub_caption(ui, "Switches the live UsdSkel clip without reloading the stage.");
                ui.add_space(style::space::BLOCK);

                for (prim_path, set) in anim_sets {
                    let key = (prim_path.clone(), set.name.clone());
                    let authored = set.selection.as_deref().unwrap_or("");
                    let current = loader_tuning
                        .variants
                        .get(&key)
                        .cloned()
                        .unwrap_or_else(|| authored.to_string());
                    let mut selected_idx =
                        set.options.iter().position(|o| o == &current).unwrap_or(0);
                    let options_str: Vec<&str> = set.options.iter().map(|s| s.as_str()).collect();

                    labelled_row(ui, prim_path.as_str(), |ui| {
                        let r = scroll_dropdown_control(
                            ui,
                            (prim_path.as_str(), "animation_clip"),
                            &mut selected_idx,
                            &options_str,
                            accent_col,
                        );
                        if r.changed() {
                            let picked = set.options[selected_idx].clone();
                            if picked != current {
                                viewport_commands.send(ViewportCommand::SetVariantSelection {
                                    prim_path: key.0.clone(),
                                    set_name: key.1.clone(),
                                    option: picked,
                                });
                            }
                        }
                    });
                }
            });
            pane.section("variants_all", "Variant sets", true, |ui| {
                let asset = Some(&*stage_info);
                match asset {
                    Some(asset) if !asset.variants.is_empty() => {
                        sub_caption(
                            ui,
                            &format!("{} prims author variant sets", asset.variants.len()),
                        );
                        ui.add_space(style::space::BLOCK);

                        egui::ScrollArea::vertical().show(ui, |ui| {
                            let mut entries: Vec<_> = asset.variants.iter().collect();
                            entries.sort_by(|a, b| a.0.cmp(b.0));
                            for (prim_path, sets) in entries {
                                nested_section(
                                    ui,
                                    prim_path.as_str(),
                                    prim_path.as_str(),
                                    accent_col,
                                    true,
                                    |ui| {
                                        for set in sets {
                                            let key = (prim_path.clone(), set.name.clone());
                                            let authored = set.selection.as_deref().unwrap_or("");
                                            let current = loader_tuning
                                                .variants
                                                .get(&key)
                                                .cloned()
                                                .unwrap_or_else(|| authored.to_string());

                                            if set.options.is_empty() {
                                                readout_row(ui, &set.name, "(no options)");
                                                continue;
                                            }

                                            let mut selected_idx = set
                                                .options
                                                .iter()
                                                .position(|o| o == &current)
                                                .unwrap_or(0);
                                            let options_str: Vec<&str> =
                                                set.options.iter().map(|s| s.as_str()).collect();

                                            labelled_row(ui, &set.name, |ui| {
                                                let r = scroll_dropdown_control(
                                                    ui,
                                                    (prim_path.as_str(), set.name.as_str()),
                                                    &mut selected_idx,
                                                    &options_str,
                                                    accent_col,
                                                );
                                                if r.changed() {
                                                    let picked = set.options[selected_idx].clone();
                                                    if picked != current {
                                                        viewport_commands.send(
                                                            ViewportCommand::SetVariantSelection {
                                                                prim_path: key.0.clone(),
                                                                set_name: key.1.clone(),
                                                                option: picked,
                                                            },
                                                        );
                                                    }
                                                }
                                            });

                                            if !current.is_empty() && current != authored {
                                                labelled_row(ui, "", |ui| {
                                                    if ui
                                                        .small_button("reset to authored")
                                                        .clicked()
                                                    {
                                                        viewport_commands.send(
                                                            ViewportCommand::ResetVariantSelection {
                                                                prim_path: key.0.clone(),
                                                                set_name: key.1.clone(),
                                                            },
                                                        );
                                                    }
                                                });
                                            }
                                        }
                                    },
                                );
                            }
                        });
                    }
                    Some(_) => {
                        sub_caption(ui, "Stage authors no variant sets.");
                    }
                    None => {
                        sub_caption(ui, "(no stage loaded yet)");
                    }
                }
            });
        },
    );
}

pub(crate) fn scroll_dropdown_control(
    ui: &mut egui::Ui,
    id_salt: impl Hash + std::fmt::Debug,
    selected: &mut usize,
    options: &[&str],
    _accent: egui::Color32,
) -> egui::Response {
    let display = options.get(*selected).copied().unwrap_or("—");
    let max_w = ui.available_width().clamp(60.0, 200.0);
    let mut changed = false;
    let mut response = egui::ComboBox::from_id_salt(("usdview_scroll_dropdown", id_salt))
        .selected_text(display)
        .width(max_w)
        .height(240.0)
        .show_ui(ui, |ui| {
            for (idx, opt) in options.iter().enumerate() {
                if ui.selectable_label(*selected == idx, *opt).clicked() {
                    if *selected != idx {
                        *selected = idx;
                        changed = true;
                    }
                    ui.close();
                }
            }
        })
        .response;
    if response.clicked() || changed {
        response.request_focus();
    }
    if response.has_focus() && !options.is_empty() {
        ui.ctx().memory_mut(|m| {
            m.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
        let delta = ui.input_mut(|i| {
            i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) as isize
                - i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) as isize
        });
        if delta != 0 {
            let len = options.len() as isize;
            let next = (*selected as isize + delta).rem_euclid(len) as usize;
            if next != *selected {
                *selected = next;
                changed = true;
                response.request_focus();
            }
        }
    }
    if changed {
        response.mark_changed();
    }
    response
}
