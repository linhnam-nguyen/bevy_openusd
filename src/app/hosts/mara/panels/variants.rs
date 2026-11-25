use std::collections::HashMap;

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;
use mara_core::vocab::{Color32 as MaraColor32, Id as MaraId};
use viewport_protocol::ViewportCommand;

use super::{ActionQueue, PanelAction, push_action};
use crate::viewport::stage::{LoaderTuning, StageInfo, VariantSetInfo};

#[derive(Clone)]
pub(super) struct VariantsSnapshot {
    skel_animation_count: usize,
    variants: HashMap<String, Vec<VariantSetInfo>>,
    overrides: HashMap<(String, String), String>,
}

pub(super) fn snapshot(world: &World) -> VariantsSnapshot {
    let info = world.resource::<StageInfo>();
    let tuning = world.resource::<LoaderTuning>();
    VariantsSnapshot {
        skel_animation_count: info.skel_animation_count,
        variants: info.variants.clone(),
        overrides: tuning.variants.clone(),
    }
}

pub(super) fn render(
    body: &mut PaneBody,
    snap: &VariantsSnapshot,
    accent: MaraColor32,
    queue: &ActionQueue,
) {
    let mut anim_sets: Vec<(&String, &VariantSetInfo)> = snap
        .variants
        .iter()
        .flat_map(|(prim, sets)| {
            sets.iter()
                .filter(|s| s.name == "anim" && !s.options.is_empty())
                .map(move |s| (prim, s))
        })
        .collect();
    anim_sets.sort_by(|a, b| a.0.cmp(b.0));

    let mut anim_pods = Vec::new();
    if anim_sets.is_empty() {
        let msg = if snap.skel_animation_count == 0 {
            "No UsdSkel animations or `anim` variant set found.".to_owned()
        } else {
            format!(
                "{} SkelAnimation prim(s) found; no `anim` variant switch.",
                snap.skel_animation_count
            )
        };
        anim_pods.push(Pod::new(MaraId::new(("mara.var", "anim_none"))).with_readout("clips", msg));
    } else {
        for (idx, (prim_path, set)) in anim_sets.into_iter().enumerate() {
            let key = (prim_path.clone(), set.name.clone());
            let authored = set.selection.as_deref().unwrap_or("");
            let current = snap
                .overrides
                .get(&key)
                .cloned()
                .unwrap_or_else(|| authored.to_owned());
            let selected_idx = set.options.iter().position(|o| o == &current).unwrap_or(0);
            let options = set.options.clone();
            let prim_owned = prim_path.clone();
            let set_owned = set.name.clone();
            let q = queue.clone();
            anim_pods.push(
                Pod::new(MaraId::new(("mara.var.anim", idx))).with_custom_units(2, move |ui| {
                    let mut sel = selected_idx;
                    let opt_refs: Vec<&str> = options.iter().map(String::as_str).collect();
                    ui.readout("prim", &prim_owned);
                    if ui
                        .dropdown(("mara.var.anim.dd", &prim_owned), &mut sel, &opt_refs)
                        .changed()
                    {
                        let picked = options[sel].clone();
                        if picked != current {
                            push_action(
                                &q,
                                PanelAction::Command(ViewportCommand::SetVariantSelection {
                                    prim_path: prim_owned,
                                    set_name: set_owned,
                                    option: picked,
                                }),
                            );
                        }
                    }
                }),
            );
        }
    }
    body.add_normal("usd.variants.anim", "Animation Clips", "branch", anim_pods);

    let mut all_pods = Vec::new();
    if snap.variants.is_empty() {
        all_pods.push(
            Pod::new(MaraId::new(("mara.var", "none")))
                .with_readout("variants", "Stage authors no variant sets."),
        );
    } else {
        let mut entries: Vec<_> = snap.variants.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (p_idx, (prim_path, sets)) in entries.into_iter().enumerate() {
            for (s_idx, set) in sets.iter().enumerate() {
                if set.options.is_empty() {
                    all_pods.push(
                        Pod::new(MaraId::new(("mara.var.empty", p_idx, s_idx)))
                            .with_readout(&set.name, "(no options)"),
                    );
                    continue;
                }
                let key = (prim_path.clone(), set.name.clone());
                let authored = set.selection.as_deref().unwrap_or("").to_owned();
                let current = snap
                    .overrides
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| authored.clone());
                let selected_idx = set.options.iter().position(|o| o == &current).unwrap_or(0);
                let options = set.options.clone();
                let prim_owned = prim_path.clone();
                let set_owned = set.name.clone();
                let can_reset = !current.is_empty() && current != authored;
                let q = queue.clone();
                all_pods.push(
                    Pod::new(MaraId::new(("mara.var.set", p_idx, s_idx))).with_custom_units(
                        if can_reset { 3 } else { 2 },
                        move |ui| {
                            let mut sel = selected_idx;
                            let opt_refs: Vec<&str> = options.iter().map(String::as_str).collect();
                            ui.readout(&prim_owned, &set_owned);
                            if ui
                                .dropdown(
                                    ("mara.var.set.dd", &prim_owned, &set_owned),
                                    &mut sel,
                                    &opt_refs,
                                )
                                .changed()
                            {
                                let picked = options[sel].clone();
                                if picked != current {
                                    push_action(
                                        &q,
                                        PanelAction::Command(ViewportCommand::SetVariantSelection {
                                            prim_path: prim_owned.clone(),
                                            set_name: set_owned.clone(),
                                            option: picked,
                                        }),
                                    );
                                }
                            }
                            if can_reset && ui.button("Reset to authored").clicked() {
                                push_action(
                                    &q,
                                    PanelAction::Command(ViewportCommand::ResetVariantSelection {
                                        prim_path: prim_owned,
                                        set_name: set_owned,
                                    }),
                                );
                            }
                        },
                    ),
                );
            }
        }
    }
    let _ = accent;
    body.add_normal("usd.variants.all", "Variant Sets", "branch", all_pods);
}
