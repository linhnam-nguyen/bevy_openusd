mod row;

use bevy::asset::Assets;
use bevy::ecs::hierarchy::Children;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use bevy_frost::prelude::*;
use bevy_frost::style;
use std::collections::HashMap;
use usd_bevy::{UsdDisplayName, UsdPrimRef};
use viewport_protocol::{FocusMode, ViewportCommand};

use crate::viewport::api::{SceneAnchorIndex, ViewportCommandInbox};
use crate::viewport::scene::SelectedPrim;
use crate::viewport::ui_frost::constants::{
    PANEL_W, RIB_TREE, RIBBON_ITEMS, RIBBONS, TreeExpanded, TreeFilter,
};
use crate::viewport::ui_frost::plugin::is_panel_open;
use row::{CtxAction, RowOutcome, draw_tree_row, queue_tree_focus, set_subtree_expanded};

#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
/// Draws the searchable USD prim hierarchy and applies row interactions.
pub fn draw_legacy_tree_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    selected: Res<SelectedPrim>,
    scene_index: Res<SceneAnchorIndex>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
    mut expanded: ResMut<TreeExpanded>,
    mut filter: ResMut<TreeFilter>,
    materials: Res<Assets<StandardMaterial>>,
    prims: Query<(Entity, &Name, &UsdPrimRef, Option<&UsdDisplayName>)>,
    mat_q: Query<&MeshMaterial3d<StandardMaterial>>,
    visibility_q: Query<(Entity, &Visibility)>,
    children: Query<&Children>,
) {
    if !is_panel_open(&open, RIB_TREE) {
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
        RIB_TREE,
        "Prim tree",
        egui::vec2(PANEL_W, 720.0),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("tree_hierarchy", "Hierarchy", true, |ui| {
                sub_caption(ui, &format!("{} prims", prims.iter().count()));
                ui.add_space(style::space::TIGHT);
                let (_, restore_selected_row) =
                    search_field_with_clear(ui, &mut filter.0, "Search prims…", accent_col);
                if restore_selected_row {
                    if let Some(selected_entity) = selected.0 {
                        if let Ok((_, _, selected_ref, _)) = prims.get(selected_entity) {
                            let segments: Vec<&str> = selected_ref
                                .path
                                .split('/')
                                .filter(|segment| !segment.is_empty())
                                .collect();

                            let mut ancestor_path = String::new();

                            for segment in segments.iter().take(segments.len().saturating_sub(1)) {
                                ancestor_path.push('/');
                                ancestor_path.push_str(segment);
                                expanded.0.insert(ancestor_path.clone(), true);
                            }
                        }
                    }
                }

                ui.add_space(style::space::BLOCK);

                let mut vis_cache: HashMap<Entity, bool> = HashMap::new();
                for (e, v) in visibility_q.iter() {
                    vis_cache.insert(e, !matches!(*v, Visibility::Hidden));
                }

                let filter_lc = filter.0.to_lowercase();
                let flat = !filter_lc.is_empty();

                let mut outcome = RowOutcome::default();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .min_scrolled_height(600.0)
                    .max_height(600.0)
                    .show(ui, |ui| {
                        if flat {
                            let mut matches: Vec<(
                                Entity,
                                &Name,
                                &UsdPrimRef,
                                Option<&UsdDisplayName>,
                            )> = prims
                                .iter()
                                .filter(|(_, name, _, display_name)| {
                                    let label = display_name
                                        .map(|display_name| display_name.0.as_str())
                                        .unwrap_or_else(|| name.as_str());

                                    label.to_lowercase().contains(&filter_lc)
                                })
                                .collect();
                            matches.sort_by(|a, b| a.2.path.cmp(&b.2.path));
                            if matches.is_empty() {
                                sub_caption(ui, "(no matches)");
                            }
                            for (entity, name, pref, dn) in &matches {
                                let sub = draw_tree_row(
                                    ui,
                                    *entity,
                                    name,
                                    pref,
                                    *dn,
                                    &prims,
                                    &mat_q,
                                    &materials,
                                    &mut vis_cache,
                                    &children,
                                    &selected,
                                    &mut expanded,
                                    accent_col,
                                    0,
                                    true,
                                    false,
                                );
                                outcome.merge(sub);
                            }
                        } else {
                            let mut roots: Vec<(
                                Entity,
                                &Name,
                                &UsdPrimRef,
                                Option<&UsdDisplayName>,
                            )> = prims
                                .iter()
                                .filter(|(_, _, pref, _)| {
                                    let p = pref.path.as_str();
                                    p.starts_with('/') && p.len() > 1 && !p[1..].contains('/')
                                })
                                .collect();
                            roots.sort_by(|a, b| a.2.path.cmp(&b.2.path));

                            if roots.is_empty() {
                                sub_caption(ui, "(no prims yet — stage loading)");
                            } else {
                                for (entity, name, pref, dn) in &roots {
                                    let sub = draw_tree_row(
                                        ui,
                                        *entity,
                                        name,
                                        pref,
                                        *dn,
                                        &prims,
                                        &mat_q,
                                        &materials,
                                        &mut vis_cache,
                                        &children,
                                        &selected,
                                        &mut expanded,
                                        accent_col,
                                        0,
                                        false,
                                        restore_selected_row,
                                    );
                                    outcome.merge(sub);
                                }
                            }
                        }
                    });

                if let Some((entity, visible)) = outcome.visibility_change {
                    if let Some(target) = scene_index.anchor_for(entity) {
                        viewport_commands
                            .send(ViewportCommand::SetSubtreeVisibility { target, visible });
                    }
                }

                if let Some(action) = outcome.ctx_action {
                    match action {
                        CtxAction::FlyTo(entity) => {
                            queue_tree_focus(
                                &mut viewport_commands,
                                &scene_index,
                                entity,
                                FocusMode::FlyToTarget,
                            );
                        }
                        CtxAction::Fit(entity) => {
                            queue_tree_focus(
                                &mut viewport_commands,
                                &scene_index,
                                entity,
                                FocusMode::FrameTarget,
                            );
                        }
                        CtxAction::ExpandDesc(entity) => {
                            set_subtree_expanded(entity, &prims, &children, &mut expanded, true);
                        }
                        CtxAction::CollapseDesc(entity) => {
                            set_subtree_expanded(entity, &prims, &children, &mut expanded, false);
                        }
                    }
                }

                if let Some(entity) = outcome.double_clicked {
                    queue_tree_focus(
                        &mut viewport_commands,
                        &scene_index,
                        entity,
                        FocusMode::FrameTarget,
                    );
                } else if let Some(entity) = outcome.clicked {
                    queue_tree_focus(
                        &mut viewport_commands,
                        &scene_index,
                        entity,
                        FocusMode::FrameTarget,
                    );
                }
            });
        },
    );
}
