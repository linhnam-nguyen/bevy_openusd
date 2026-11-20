use std::collections::HashSet;

use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;
use mara_core::vocab::{Color32 as MaraColor32, Id as MaraId};
use usd_bevy::UsdPrimRef;

use super::super::MaraPageState;
use super::{ActionQueue, PanelAction, page_window, push_action};

pub(in crate::viewport::app::hosts::mara) const MATERIALS_PAGE_SIZE: usize = 24;

#[derive(Clone)]
pub(in crate::viewport::app::hosts::mara) struct MaterialEntry {
    pub(in crate::viewport::app::hosts::mara) id: AssetId<StandardMaterial>,
    pub(in crate::viewport::app::hosts::mara) label: String,
    pub(in crate::viewport::app::hosts::mara) base_color: [f32; 3],
    pub(in crate::viewport::app::hosts::mara) roughness: f32,
    pub(in crate::viewport::app::hosts::mara) metallic: f32,
}

#[derive(Clone)]
pub(in crate::viewport::app::hosts::mara) struct MaterialsSnapshot {
    pub(in crate::viewport::app::hosts::mara) entries: Vec<MaterialEntry>,
    pub(in crate::viewport::app::hosts::mara) material_page: usize,
}

pub(in crate::viewport::app::hosts::mara) fn snapshot(
    world: &World,
    pages: &MaraPageState,
) -> MaterialsSnapshot {
    let mut bound: HashSet<AssetId<StandardMaterial>> = HashSet::new();
    for entity in world.iter_entities() {
        if entity.contains::<UsdPrimRef>()
            && let Some(mm) = entity.get::<MeshMaterial3d<StandardMaterial>>()
        {
            bound.insert(mm.0.id());
        }
    }

    let asset_server = world.get_resource::<AssetServer>();
    let materials = world.get_resource::<Assets<StandardMaterial>>();
    let mut entries = Vec::new();
    if let Some(materials) = materials {
        for (id, mat) in materials.iter() {
            if !bound.contains(&id) {
                continue;
            }
            let label = asset_server
                .and_then(|srv| srv.get_path(id))
                .map(|p| p.to_string())
                .unwrap_or_else(|| format!("{id:?}"));
            let linear = mat.base_color.to_linear();
            entries.push(MaterialEntry {
                id,
                label,
                base_color: [linear.red, linear.green, linear.blue],
                roughness: mat.perceptual_roughness,
                metallic: mat.metallic,
            });
        }
    }
    entries.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.id.cmp(&b.id)));
    MaterialsSnapshot {
        entries,
        material_page: pages.material_page,
    }
}

pub(super) fn render(
    body: &mut PaneBody,
    snap: &MaterialsSnapshot,
    accent: MaraColor32,
    queue: &ActionQueue,
) {
    let win = page_window(
        snap.entries.len(),
        snap.material_page,
        MATERIALS_PAGE_SIZE,
    );
    let mut pods = vec![
        Pod::new(MaraId::new(("mara.mat", "readonly_notice"))).with_custom_units(2, |ui| {
            ui.label(
                "Read-only rendered material. Authoritative USD material editing is not available in this native panel.",
            );
        }),
        Pod::new(MaraId::new(("mara.mat", "count")))
            .with_readout("bound", snap.entries.len().to_string()),
        Pod::new(MaraId::new(("mara.mat", "range")))
            .with_readout("displaying", win.range_label()),
    ];

    if snap.entries.is_empty() {
        pods.push(
            Pod::new(MaraId::new(("mara.mat", "empty")))
                .with_readout("status", "No bound rendered materials"),
        );
    } else {
        for entry in &snap.entries[win.start..win.end] {
            let short = entry
                .label
                .rsplit('/')
                .next()
                .unwrap_or(&entry.label)
                .chars()
                .take(40)
                .collect::<String>();
            let title = format!("{short} — rendered material (read-only)");
            let section_key = format!("{:?}", entry.id);
            let rgb = entry.base_color;
            let rough = entry.roughness;
            let metal = entry.metallic;
            pods.push(
                Pod::new(MaraId::new(("mara.mat.entry", entry.id))).with_custom_units(4, move |ui| {
                    ui.section(&title, &section_key, false, |ui| {
                        ui.readout(
                            "Color (linear RGB)",
                            &format!("{:.3}, {:.3}, {:.3}", rgb[0], rgb[1], rgb[2]),
                        );
                        ui.readout("Roughness", &format!("{rough:.3}"));
                        ui.readout("Metallic", &format!("{metal:.3}"));
                    });
                }),
            );
        }

        if win.total_pages > 1 {
            let q_nav = queue.clone();
            let page = win.page;
            let total_pages = win.total_pages;
            pods.push(
                Pod::new(MaraId::new(("mara.mat", "pager"))).with_custom_units(1, move |ui| {
                    ui.horizontal(|ui| {
                        if page > 0 && ui.button("Previous").clicked() {
                            push_action(&q_nav, PanelAction::SetMaterialPage(page - 1));
                        }
                        if page + 1 < total_pages && ui.button("Next").clicked() {
                            push_action(&q_nav, PanelAction::SetMaterialPage(page + 1));
                        }
                    });
                }),
            );
        }
    }

    let _ = accent;
    body.add_normal("usd.materials", "Materials (Read-Only)", "color", pods);
}
