use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;
use mara_core::vocab::{Color32 as MaraColor32, Id as MaraId};
use mara_core::widget::{TreeBody, TreeIconKind, TreeIconSlot};
use viewport_protocol::{FocusMode, PrimNodeReadModel, SceneAnchor, SceneSearchMatch, ViewportCommand};

use super::{ActionQueue, PanelAction, push_action};
use crate::api::ViewportReadModelState;

const TREE_DEFAULT_OPEN_DEPTH: u32 = 2;

#[derive(Default)]
pub(in crate::app::hosts::mara) struct TreeUiState {
    pub(super) filter: String,
    pub(super) expanded: HashMap<SceneAnchor, bool>,
}

#[derive(Clone)]
pub(super) struct TreeSnapshot {
    total_prims: u32,
    filter: String,
    selection: Option<SceneAnchor>,
    nodes: Vec<PrimNodeReadModel>,
    search_matches: Vec<SceneSearchMatch>,
    search_status: Option<(u32, bool)>,
    expanded: HashMap<SceneAnchor, bool>,
}

pub(super) fn snapshot(world: &World, state: &TreeUiState) -> TreeSnapshot {
    let rm = world.resource::<ViewportReadModelState>();
    let total_prims = rm
        .snapshot()
        .map(|s| s.scene.total_prims)
        .unwrap_or_default();
    let selection = rm.snapshot().and_then(|s| s.selection.primary.clone());
    TreeSnapshot {
        total_prims,
        filter: state.filter.clone(),
        selection,
        nodes: rm.scene_nodes(),
        search_matches: rm.search_results().to_vec(),
        search_status: rm.search_status(),
        expanded: state.expanded.clone(),
    }
}

pub(super) fn render(
    body: &mut PaneBody,
    snap: &TreeSnapshot,
    accent: MaraColor32,
    queue: &ActionQueue,
) {
    let mut children_by_parent: HashMap<SceneAnchor, Vec<PrimNodeReadModel>> = HashMap::new();
    let mut roots = Vec::new();
    for node in &snap.nodes {
        if let Some(parent) = &node.parent {
            children_by_parent
                .entry(parent.clone())
                .or_default()
                .push(node.clone());
        } else {
            roots.push(node.clone());
        }
    }
    roots.sort_by(|a, b| a.anchor.prim_path.cmp(&b.anchor.prim_path));
    for children in children_by_parent.values_mut() {
        children.sort_by(|a, b| a.anchor.prim_path.cmp(&b.anchor.prim_path));
    }

    let filter_trimmed = snap.filter.trim().to_owned();
    let mut filter_draft = snap.filter.clone();
    let q_filter = queue.clone();
    let q_tree = queue.clone();
    let q_more = queue.clone();
    let snap_tree = snap.clone();

    let mut pods = vec![
        Pod::new(MaraId::new(("mara.tree", "count")))
            .with_readout("prims", snap.total_prims.to_string()),
        Pod::new(MaraId::new(("mara.tree", "search"))).with_custom_units(2, move |ui| {
            let prev = filter_draft.clone();
            ui.text_input(&mut filter_draft, "Search prims…");
            if filter_draft != prev {
                push_action(&q_filter, PanelAction::TreeSearch(filter_draft));
            }
        }),
        Pod::new(MaraId::new(("mara.tree", "hierarchy")))
            .fill()
            .with_tree(10, move |tree| {
                let mut expanded = snap_tree.expanded.clone();
                if !filter_trimmed.is_empty() {
                    for m in &snap_tree.search_matches {
                        let node = PrimNodeReadModel {
                            anchor: m.anchor.clone(),
                            parent: m.parent.clone(),
                            label: m.label.clone(),
                            display_name: None,
                            visible: m.visible,
                            has_children: m.has_children,
                        };
                        draw_tree_row(
                            tree,
                            &node,
                            &children_by_parent,
                            snap_tree.selection.as_ref(),
                            &mut expanded,
                            accent,
                            0,
                            true,
                            &q_tree,
                        );
                    }
                } else {
                    for root in &roots {
                        draw_tree_row(
                            tree,
                            root,
                            &children_by_parent,
                            snap_tree.selection.as_ref(),
                            &mut expanded,
                            accent,
                            0,
                            false,
                            &q_tree,
                        );
                    }
                }
            }),
    ];

    if !snap.filter.trim().is_empty()
        && let Some((total, has_more)) = snap.search_status
    {
        pods.push(
            Pod::new(MaraId::new(("mara.tree", "search_count"))).with_readout(
                "matches",
                format!("{} / {}", snap.search_matches.len(), total),
            ),
        );
        if has_more {
            pods.push(
                Pod::new(MaraId::new(("mara.tree", "search_more"))).with_custom_units(1, move |ui| {
                    if ui.button("Load more results").clicked() {
                        push_action(&q_more, PanelAction::TreeLoadMoreSearch);
                    }
                }),
            );
        }
    }

    body.add_normal("usd.tree", "Hierarchy", "cube-tree", pods);
}

#[allow(clippy::too_many_arguments)]
fn draw_tree_row(
    tree: &mut TreeBody,
    node: &PrimNodeReadModel,
    children_by_parent: &HashMap<SceneAnchor, Vec<PrimNodeReadModel>>,
    selection: Option<&SceneAnchor>,
    expanded: &mut HashMap<SceneAnchor, bool>,
    accent: MaraColor32,
    depth: u32,
    leaf_override: bool,
    queue: &ActionQueue,
) {
    let has_children = !leaf_override && node.has_children;
    let is_selected = selection == Some(&node.anchor);
    let mut visible = node.visible;
    let visible_before = visible;
    let mut slots = [
        TreeIconSlot::new(TreeIconKind::Eye, &mut visible).with_tooltip("Toggle visibility")
    ];
    let mut open = if has_children {
        *expanded
            .entry(node.anchor.clone())
            .or_insert(depth < TREE_DEFAULT_OPEN_DEPTH)
    } else {
        false
    };
    let mut hasher = std::hash::DefaultHasher::new();
    node.anchor.hash(&mut hasher);
    let row_id = hasher.finish() as usize;

    let resp = tree.row(
        row_id,
        depth,
        has_children.then_some(&mut open),
        Some("cube"),
        &node.label,
        is_selected,
        accent,
        &mut slots,
    );

    if has_children && expanded.get(&node.anchor).copied() != Some(open) {
        expanded.insert(node.anchor.clone(), open);
        push_action(queue, PanelAction::TreeExpand(node.anchor.clone(), open));
    }
    if visible != visible_before {
        push_action(
            queue,
            PanelAction::Command(ViewportCommand::SetSubtreeVisibility {
                target: node.anchor.clone(),
                visible,
            }),
        );
    }
    if resp.body.clicked || resp.body.double_clicked {
        push_action(
            queue,
            PanelAction::Command(ViewportCommand::FocusTarget {
                target: node.anchor.clone(),
                mode: FocusMode::FrameTarget,
            }),
        );
    }

    if has_children
        && open
        && let Some(children) = children_by_parent.get(&node.anchor)
    {
        for child in children {
            draw_tree_row(
                tree,
                child,
                children_by_parent,
                selection,
                expanded,
                accent,
                depth.saturating_add(1),
                false,
                queue,
            );
        }
    }
}
