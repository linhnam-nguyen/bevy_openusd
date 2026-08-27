//! Frost presentation adapter for the public scene-tree read model.
//!
//! This module deliberately knows only protocol identities and events. It
//! does not query entities, hierarchy, materials, or visibility from Bevy.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use bevy_frost::prelude::*;
use bevy_frost::style;
use viewport_protocol::{
    DEFAULT_SCENE_SEARCH_PAGE_SIZE, FocusMode, PrimNodeReadModel, SceneAnchor, ViewportCommand,
};

use super::{
    PANEL_W, RIB_TREE, RIBBON_ITEMS, RIBBONS, TREE_DEFAULT_OPEN_DEPTH, TreeFilter, is_panel_open,
};
use crate::viewport::api::{ViewportCommandInbox, ViewportReadModelState};

/// Presentation-only expansion state for the renderer-neutral tree.
#[derive(Resource, Default)]
pub(crate) struct ProtocolTreeExpanded(pub(crate) HashMap<SceneAnchor, bool>);

#[derive(Default)]
struct TreeOutcome {
    focus: Option<(SceneAnchor, FocusMode)>,
    visibility_change: Option<(SceneAnchor, bool)>,
    expanded: Vec<SceneAnchor>,
    search_changed: Option<String>,
    load_more_search: bool,
}

/// Draws the delivered Frost tree from the same paged public read model a
/// remote frontend receives. The retained legacy implementation in `mod.rs`
/// is intentionally not registered.
pub(crate) fn draw_tree_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    mut read_model: ResMut<ViewportReadModelState>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
    mut expanded: ResMut<ProtocolTreeExpanded>,
    mut filter: ResMut<TreeFilter>,
) {
    if !is_panel_open(&open, RIB_TREE) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let nodes = read_model.scene_nodes();
    let selection = read_model
        .snapshot()
        .and_then(|snapshot| snapshot.selection.primary.clone());
    let total_prims = read_model
        .snapshot()
        .map(|snapshot| snapshot.scene.total_prims)
        .unwrap_or_default();
    let search_matches = read_model.search_results().to_vec();
    let search_status = read_model.search_status();
    let accent_col = accent.0;
    let mut keep = true;
    let mut outcome = TreeOutcome::default();

    let mut children_by_parent: HashMap<SceneAnchor, Vec<PrimNodeReadModel>> = HashMap::new();
    let mut roots = Vec::new();
    for node in &nodes {
        if let Some(parent) = &node.parent {
            children_by_parent
                .entry(parent.clone())
                .or_default()
                .push(node.clone());
        } else {
            roots.push(node.clone());
        }
    }
    roots.sort_by(|left, right| left.anchor.prim_path.cmp(&right.anchor.prim_path));
    for children in children_by_parent.values_mut() {
        children.sort_by(|left, right| left.anchor.prim_path.cmp(&right.anchor.prim_path));
    }

    floating_window_for_item(
        ctx,
        RIBBONS,
        RIBBON_ITEMS,
        &placement,
        RIB_TREE,
        "Prim tree",
        egui::vec2(PANEL_W, 720.0),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("tree_hierarchy", "Hierarchy", true, |ui| {
                sub_caption(ui, &format!("{total_prims} prims"));
                ui.add_space(style::space::TIGHT);
                let (search_response, restore_selected_row) =
                    search_field_with_clear(ui, &mut filter.0, "Search prims…", accent_col);
                if search_response.changed() || restore_selected_row {
                    outcome.search_changed = Some(filter.0.trim().to_owned());
                    if restore_selected_row {
                        expand_loaded_selection_ancestors(
                            selection.as_ref(),
                            &nodes,
                            &mut expanded,
                        );
                    }
                }

                ui.add_space(style::space::BLOCK);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .min_scrolled_height(600.0)
                    .max_height(600.0)
                    .show(ui, |ui| {
                        if !filter.0.trim().is_empty() {
                            let found = search_matches.len();
                            let total = search_status.map(|(total, _)| total).unwrap_or_default();
                            sub_caption(ui, &format!("{found} of {total} matching prims"));
                            if search_matches.is_empty() {
                                sub_caption(ui, "(searching or no matches)");
                            }
                            for search in &search_matches {
                                let node = PrimNodeReadModel {
                                    anchor: search.anchor.clone(),
                                    parent: search.parent.clone(),
                                    label: search.label.clone(),
                                    display_name: None,
                                    visible: search.visible,
                                    has_children: search.has_children,
                                };
                                draw_protocol_tree_row(
                                    ui,
                                    &node,
                                    &children_by_parent,
                                    selection.as_ref(),
                                    &mut expanded,
                                    accent_col,
                                    0,
                                    true,
                                    false,
                                    &mut outcome,
                                );
                            }
                            if search_status.is_some_and(|(_, has_more)| has_more)
                                && wide_button(ui, "Load more results", accent_col).clicked()
                            {
                                outcome.load_more_search = true;
                            }
                        } else if roots.is_empty() {
                            sub_caption(ui, "(no prims yet — stage loading)");
                        } else {
                            for root in &roots {
                                draw_protocol_tree_row(
                                    ui,
                                    root,
                                    &children_by_parent,
                                    selection.as_ref(),
                                    &mut expanded,
                                    accent_col,
                                    0,
                                    false,
                                    restore_selected_row,
                                    &mut outcome,
                                );
                            }
                        }
                    });
            });
        },
    );

    if let Some(query) = outcome.search_changed {
        if query.is_empty() {
            read_model.clear_search();
        } else {
            let request_id = viewport_commands.send(ViewportCommand::SearchScene {
                query: query.clone(),
                offset: 0,
                limit: DEFAULT_SCENE_SEARCH_PAGE_SIZE,
            });
            read_model.begin_search(request_id, query);
        }
    }
    if outcome.load_more_search
        && let Some((query, offset)) = read_model.next_search_page()
    {
        viewport_commands.send(ViewportCommand::SearchScene {
            query,
            offset,
            limit: DEFAULT_SCENE_SEARCH_PAGE_SIZE,
        });
    }
    let mut requested = HashSet::new();
    for parent in outcome.expanded {
        if requested.insert(parent.clone()) {
            read_model.request_scene_children(parent);
        }
    }
    for request in read_model.take_scene_page_requests() {
        viewport_commands.send(ViewportCommand::RequestSceneChildren {
            parent: request.parent,
            page: request.page,
            page_size: request.page_size,
        });
    }
    if let Some((target, visible)) = outcome.visibility_change {
        viewport_commands.send(ViewportCommand::SetSubtreeVisibility { target, visible });
    }
    if let Some((target, mode)) = outcome.focus {
        viewport_commands.send(ViewportCommand::FocusTarget { target, mode });
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_protocol_tree_row(
    ui: &mut egui::Ui,
    node: &PrimNodeReadModel,
    children_by_parent: &HashMap<SceneAnchor, Vec<PrimNodeReadModel>>,
    selection: Option<&SceneAnchor>,
    expanded: &mut ProtocolTreeExpanded,
    accent: egui::Color32,
    depth: u32,
    leaf_override: bool,
    scroll_selected_to_top: bool,
    outcome: &mut TreeOutcome,
) {
    let has_children = !leaf_override && node.has_children;
    let is_selected = selection == Some(&node.anchor);
    let mut visible = node.visible;
    let visible_before = visible;
    let mut slots =
        vec![TreeIconSlot::new(TreeIconKind::Eye, &mut visible).with_tooltip("Toggle visibility")];

    let mut open = if has_children {
        *expanded
            .0
            .entry(node.anchor.clone())
            .or_insert(depth < TREE_DEFAULT_OPEN_DEPTH)
    } else {
        false
    };
    let row_id_salt = {
        let mut hasher = std::hash::DefaultHasher::new();
        node.anchor.hash(&mut hasher);
        hasher.finish()
    };
    let response = tree_row(
        ui,
        row_id_salt,
        depth,
        has_children.then_some(&mut open),
        None,
        &node.label,
        is_selected,
        accent,
        &mut slots,
    );
    if has_children {
        expanded.0.insert(node.anchor.clone(), open);
        if open {
            outcome.expanded.push(node.anchor.clone());
        }
    }
    if scroll_selected_to_top && is_selected {
        response.body.scroll_to_me(Some(egui::Align::TOP));
    }
    if visible != visible_before {
        outcome.visibility_change = Some((node.anchor.clone(), visible));
    }
    if response.body.hovered() {
        response.body.clone().on_hover_text(&node.anchor.prim_path);
    }
    if response.body.double_clicked() || response.body.clicked() {
        outcome.focus = Some((node.anchor.clone(), FocusMode::FrameTarget));
    }

    context_menu_frost(&response.body, accent, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        if wide_button(ui, "Fly to", accent).clicked() {
            outcome.focus = Some((node.anchor.clone(), FocusMode::FlyToTarget));
            ui.close();
        }
        if wide_button(ui, "Fit to bounds", accent).clicked() {
            outcome.focus = Some((node.anchor.clone(), FocusMode::FrameTarget));
            ui.close();
        }
        if wide_button(ui, "Copy path", accent).clicked() {
            ui.ctx().copy_text(node.anchor.prim_path.clone());
            ui.close();
        }
        if has_children && wide_button(ui, "Expand descendants", accent).clicked() {
            outcome.expanded.extend(set_loaded_subtree_expanded(
                &node.anchor,
                children_by_parent,
                expanded,
                true,
            ));
            ui.close();
        }
        if has_children && wide_button(ui, "Collapse descendants", accent).clicked() {
            set_loaded_subtree_expanded(&node.anchor, children_by_parent, expanded, false);
            ui.close();
        }
    });

    if has_children
        && open
        && let Some(children) = children_by_parent.get(&node.anchor)
    {
        for child in children {
            draw_protocol_tree_row(
                ui,
                child,
                children_by_parent,
                selection,
                expanded,
                accent,
                depth.saturating_add(1),
                false,
                scroll_selected_to_top,
                outcome,
            );
        }
    }
}

fn set_loaded_subtree_expanded(
    root: &SceneAnchor,
    children_by_parent: &HashMap<SceneAnchor, Vec<PrimNodeReadModel>>,
    expanded: &mut ProtocolTreeExpanded,
    open: bool,
) -> Vec<SceneAnchor> {
    let mut changed = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(anchor) = pending.pop() {
        expanded.0.insert(anchor.clone(), open);
        if open {
            changed.push(anchor.clone());
        }
        if let Some(children) = children_by_parent.get(&anchor) {
            pending.extend(children.iter().map(|child| child.anchor.clone()));
        }
    }
    changed
}

fn expand_loaded_selection_ancestors(
    selection: Option<&SceneAnchor>,
    nodes: &[PrimNodeReadModel],
    expanded: &mut ProtocolTreeExpanded,
) {
    let Some(selection) = selection else {
        return;
    };
    let parents: HashMap<_, _> = nodes
        .iter()
        .map(|node| (node.anchor.clone(), node.parent.clone()))
        .collect();
    let mut current = parents.get(selection).cloned().flatten();
    while let Some(anchor) = current {
        current = parents.get(&anchor).cloned().flatten();
        expanded.0.insert(anchor, true);
    }
}
