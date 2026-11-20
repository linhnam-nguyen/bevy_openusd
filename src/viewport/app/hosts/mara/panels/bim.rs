use std::collections::HashMap;

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;
use mara_core::vocab::{Color32 as MaraColor32, Id as MaraId};
use viewport_protocol::{
    BimClassificationFieldCatalogue, BimPropertiesReadModel, BimPropertyGroupId, CommonValue,
    HierarchySource, ViewportCommand,
};

use super::super::MaraPageState;
use super::{ActionQueue, PanelAction, page_window, push_action};
use crate::viewport::api::{
    BimCatalogueLoadState, BimPropertiesLoadState, ViewportReadModelState,
};
use crate::viewport::scene::ActiveHierarchyProvider;

pub(in crate::viewport::app::hosts::mara) const CATALOGUE_PAGE_SIZE: usize = 12;
pub(in crate::viewport::app::hosts::mara) const PROPERTY_GROUP_PAGE_SIZE: usize = 16;

#[derive(Clone)]
pub(in crate::viewport::app::hosts::mara) enum BimPropertiesViewState {
    StageNotLoaded,
    NoSelection,
    NotRequested,
    Loading { received_pages: u32, total_pages: u32 },
    Error(String),
    Ready(BimPropertiesReadModel),
}

#[derive(Clone)]
pub(in crate::viewport::app::hosts::mara) enum BimCatalogueViewState {
    StageNotLoaded,
    NotRequested,
    Loading { received_pages: u32, total_pages: u32 },
    Error(String),
    Ready(BimClassificationFieldCatalogue),
}

#[derive(Clone)]
pub(in crate::viewport::app::hosts::mara) struct BimSnapshot {
    pub(in crate::viewport::app::hosts::mara) hierarchy_source: HierarchySource,
    pub(in crate::viewport::app::hosts::mara) properties: BimPropertiesViewState,
    pub(in crate::viewport::app::hosts::mara) catalogue: BimCatalogueViewState,
    pub(in crate::viewport::app::hosts::mara) catalogue_page: usize,
    pub(in crate::viewport::app::hosts::mara) property_pages: HashMap<BimPropertyGroupId, usize>,
}

pub(in crate::viewport::app::hosts::mara) fn snapshot(world: &World, pages: &MaraPageState) -> BimSnapshot {
    let rm = world.resource::<ViewportReadModelState>();
    let provider = world.resource::<ActiveHierarchyProvider>();
    let snap = rm.snapshot();

    let properties = match rm.bim_properties_state() {
        BimPropertiesLoadState::StageNotLoaded => BimPropertiesViewState::StageNotLoaded,
        BimPropertiesLoadState::NoSelection => BimPropertiesViewState::NoSelection,
        BimPropertiesLoadState::NotRequested => BimPropertiesViewState::NotRequested,
        BimPropertiesLoadState::Loading {
            received_pages,
            total_pages,
        } => BimPropertiesViewState::Loading {
            received_pages,
            total_pages,
        },
        BimPropertiesLoadState::Error(msg) => BimPropertiesViewState::Error(msg.to_owned()),
        BimPropertiesLoadState::Ready(props) => {
            // Defense-in-depth identity check against active snapshot.
            let valid = snap.is_some_and(|s| {
                s.stage.loaded
                    && !s.selection.targets.is_empty()
                    && props.targets == s.selection.targets
                    && props.selection_revision == s.selection_revision
            });
            if valid {
                BimPropertiesViewState::Ready(props.clone())
            } else {
                BimPropertiesViewState::NotRequested
            }
        }
    };

    let catalogue = match rm.bim_catalogue_state() {
        BimCatalogueLoadState::StageNotLoaded => BimCatalogueViewState::StageNotLoaded,
        BimCatalogueLoadState::NotRequested => BimCatalogueViewState::NotRequested,
        BimCatalogueLoadState::Loading {
            received_pages,
            total_pages,
        } => BimCatalogueViewState::Loading {
            received_pages,
            total_pages,
        },
        BimCatalogueLoadState::Error(msg) => BimCatalogueViewState::Error(msg.to_owned()),
        BimCatalogueLoadState::Ready(cat) => {
            if snap.is_some_and(|s| s.stage.loaded) {
                BimCatalogueViewState::Ready(cat.clone())
            } else {
                BimCatalogueViewState::StageNotLoaded
            }
        }
    };

    BimSnapshot {
        hierarchy_source: provider.source(),
        properties,
        catalogue,
        catalogue_page: pages.catalogue_page,
        property_pages: pages.property_pages.clone(),
    }
}

pub(super) fn render(
    body: &mut PaneBody,
    snap: &BimSnapshot,
    accent: MaraColor32,
    queue: &ActionQueue,
) {
    let q_req_props = queue.clone();
    let q_req_cat = queue.clone();
    let q_prim = queue.clone();
    let q_clear_recipe = queue.clone();
    let is_bim = matches!(snap.hierarchy_source, HierarchySource::BimClassification);

    body.add_normal(
        "usd.bim.actions",
        "BIM & Classification",
        "options",
        vec![
            Pod::new(MaraId::new(("mara.bim", "hier_mode"))).with_readout(
                "hierarchy",
                if is_bim {
                    "BIM Classification"
                } else {
                    "USD Prim"
                },
            ),
            Pod::new(MaraId::new(("mara.bim", "btns"))).with_custom_units(4, move |ui| {
                if ui.button("Read BIM properties for selection").clicked() {
                    push_action(
                        &q_req_props,
                        PanelAction::Command(ViewportCommand::RequestBimProperties),
                    );
                }
                if ui.button("Refresh classification field catalogue").clicked() {
                    push_action(
                        &q_req_cat,
                        PanelAction::Command(
                            ViewportCommand::RequestBimClassificationFieldCatalogue {
                                known_revision: None,
                            },
                        ),
                    );
                }
                if is_bim && ui.button("Switch to USD Prim hierarchy").clicked() {
                    push_action(
                        &q_prim,
                        PanelAction::Command(ViewportCommand::SetHierarchySource {
                            source: HierarchySource::Prim,
                            classification_recipe: None,
                        }),
                    );
                }
                if ui.button("Clear classification colours").clicked() {
                    push_action(
                        &q_clear_recipe,
                        PanelAction::Command(ViewportCommand::SetBimClassificationRecipe {
                            recipe: None,
                        }),
                    );
                }
            }),
        ],
    );

    match &snap.catalogue {
        BimCatalogueViewState::StageNotLoaded => {
            body.add_normal(
                "usd.bim.catalogue",
                "Field Catalogue",
                "list",
                vec![
                    Pod::new(MaraId::new(("mara.bim", "cat_status")))
                        .with_readout("status", "Stage is not loaded"),
                ],
            );
        }
        BimCatalogueViewState::NotRequested => {
            body.add_normal(
                "usd.bim.catalogue",
                "Field Catalogue",
                "list",
                vec![
                    Pod::new(MaraId::new(("mara.bim", "cat_status")))
                        .with_readout("status", "Not loaded — click Refresh"),
                ],
            );
        }
        BimCatalogueViewState::Loading {
            received_pages,
            total_pages,
        } => {
            let status = if *total_pages > 0 {
                format!("Loading catalogue ({received_pages}/{total_pages} pages)…")
            } else {
                "Loading catalogue…".to_owned()
            };
            body.add_normal(
                "usd.bim.catalogue",
                "Field Catalogue",
                "list",
                vec![
                    Pod::new(MaraId::new(("mara.bim", "cat_status")))
                        .with_readout("status", status),
                ],
            );
        }
        BimCatalogueViewState::Error(msg) => {
            body.add_normal(
                "usd.bim.catalogue",
                "Field Catalogue",
                "list",
                vec![
                    Pod::new(MaraId::new(("mara.bim", "cat_error")))
                        .with_readout("error", msg.clone()),
                ],
            );
        }
        BimCatalogueViewState::Ready(cat) => {
            let win = page_window(cat.fields.len(), snap.catalogue_page, CATALOGUE_PAGE_SIZE);
            let mut cat_pods = vec![
                Pod::new(MaraId::new(("mara.bim", "cat_rev")))
                    .with_readout("semantic_rev", cat.semantic_revision.to_string()),
                Pod::new(MaraId::new(("mara.bim", "cat_count")))
                    .with_readout("fields", cat.fields.len().to_string()),
                Pod::new(MaraId::new(("mara.bim", "cat_range")))
                    .with_readout("displaying", win.range_label()),
            ];
            if cat.fields.is_empty() {
                cat_pods.push(
                    Pod::new(MaraId::new(("mara.bim", "cat_empty")))
                        .with_readout("status", "No classification fields"),
                );
            } else {
                for field in &cat.fields[win.start..win.end] {
                    cat_pods.push(
                        Pod::new(MaraId::new(("mara.bim", "cat_field", field.key.stable_key())))
                            .with_readout(&field.label, format!("{:?}", field.scope)),
                    );
                }
                if win.total_pages > 1 {
                    let q_cat_nav = queue.clone();
                    let page = win.page;
                    let total_pages = win.total_pages;
                    cat_pods.push(
                        Pod::new(MaraId::new(("mara.bim", "cat_pager"))).with_custom_units(
                            1,
                            move |ui| {
                                ui.horizontal(|ui| {
                                    if page > 0 && ui.button("Previous").clicked() {
                                        push_action(
                                            &q_cat_nav,
                                            PanelAction::SetCataloguePage(page - 1),
                                        );
                                    }
                                    if page + 1 < total_pages && ui.button("Next").clicked() {
                                        push_action(
                                            &q_cat_nav,
                                            PanelAction::SetCataloguePage(page + 1),
                                        );
                                    }
                                });
                            },
                        ),
                    );
                }
            }
            body.add_normal("usd.bim.catalogue", "Field Catalogue", "list", cat_pods);
        }
    }

    match &snap.properties {
        BimPropertiesViewState::StageNotLoaded => {
            body.add_normal(
                "usd.bim.empty",
                "BIM Properties",
                "options",
                vec![
                    Pod::new(MaraId::new(("mara.bim", "none")))
                        .with_readout("status", "Stage is not loaded"),
                ],
            );
        }
        BimPropertiesViewState::NoSelection => {
            body.add_normal(
                "usd.bim.empty",
                "BIM Properties",
                "options",
                vec![
                    Pod::new(MaraId::new(("mara.bim", "none")))
                        .with_readout("status", "Select a prim and click Read BIM properties"),
                ],
            );
        }
        BimPropertiesViewState::NotRequested => {
            body.add_normal(
                "usd.bim.empty",
                "BIM Properties",
                "options",
                vec![
                    Pod::new(MaraId::new(("mara.bim", "none"))).with_readout(
                        "status",
                        "Not requested for current selection — click Read BIM properties",
                    ),
                ],
            );
        }
        BimPropertiesViewState::Loading {
            received_pages,
            total_pages,
        } => {
            let status = if *total_pages > 0 {
                format!("Loading properties ({received_pages}/{total_pages} pages)…")
            } else {
                "Loading properties…".to_owned()
            };
            body.add_normal(
                "usd.bim.loading",
                "BIM Properties",
                "options",
                vec![
                    Pod::new(MaraId::new(("mara.bim", "loading")))
                        .with_readout("status", status),
                ],
            );
        }
        BimPropertiesViewState::Error(msg) => {
            body.add_normal(
                "usd.bim.error",
                "BIM Properties",
                "options",
                vec![
                    Pod::new(MaraId::new(("mara.bim", "error")))
                        .with_readout("error", msg.clone()),
                ],
            );
        }
        BimPropertiesViewState::Ready(props) => {
            let mut prop_pods = vec![
                Pod::new(MaraId::new(("mara.bim", "target_count")))
                    .with_readout("targets", props.targets.len().to_string()),
            ];
            if props.groups.is_empty() {
                prop_pods.push(
                    Pod::new(MaraId::new(("mara.bim", "props_empty")))
                        .with_readout("status", "No BIM properties for selected prim(s)"),
                );
            }
            for group in &props.groups {
                let desired_page = snap.property_pages.get(&group.id).copied().unwrap_or(0);
                let win = page_window(
                    group.properties.len(),
                    desired_page,
                    PROPERTY_GROUP_PAGE_SIZE,
                );
                let group_name = group.name.clone();
                let range_label = win.range_label();
                let group_id = group.id;
                let rows: Vec<(String, String)> = group.properties[win.start..win.end]
                    .iter()
                    .map(|p| {
                        let val = match &p.value {
                            CommonValue::Same(v) => format!("{v:?}"),
                            CommonValue::Multiple => "<multiple>".to_owned(),
                        };
                        (p.label.clone(), val)
                    })
                    .collect();
                let has_pager = win.total_pages > 1;
                let page = win.page;
                let total_pages = win.total_pages;
                let units = rows.len().saturating_add(1 + usize::from(has_pager)).max(1);
                let q_group = queue.clone();
                let section_key = format!("{:?}", group.id);
                prop_pods.push(
                    Pod::new(MaraId::new(("mara.bim", "group", group.id))).with_custom_units(
                        units,
                        move |ui| {
                            ui.section(&group_name, &section_key, true, |ui| {
                                ui.readout("displaying", &range_label);
                                if rows.is_empty() {
                                    ui.readout("status", "No properties in group");
                                } else {
                                    for (k, v) in &rows {
                                        ui.readout(k, v);
                                    }
                                }
                                if has_pager {
                                    ui.horizontal(|ui| {
                                        if page > 0 && ui.button("Previous").clicked() {
                                            push_action(
                                                &q_group,
                                                PanelAction::SetPropertyPage {
                                                    group_id,
                                                    page: page - 1,
                                                },
                                            );
                                        }
                                        if page + 1 < total_pages && ui.button("Next").clicked() {
                                            push_action(
                                                &q_group,
                                                PanelAction::SetPropertyPage {
                                                    group_id,
                                                    page: page + 1,
                                                },
                                            );
                                        }
                                    });
                                }
                            });
                        },
                    ),
                );
            }
            body.add_normal("usd.bim.properties", "BIM Properties", "options", prop_pods);
        }
    }
    let _ = accent;
}
