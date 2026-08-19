use bevy::asset::Assets;
use bevy::ecs::hierarchy::Children;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy_egui::egui;
use bevy_frost::prelude::*;
use bevy_frost::style;
use std::collections::HashMap;
use usd_bevy::{UsdDisplayName, UsdPrimRef};
use viewport_protocol::{FocusMode, ViewportCommand};

use crate::viewport::api::{SceneAnchorIndex, ViewportCommandInbox};
use crate::viewport::scene::SelectedPrim;
use crate::viewport::ui_frost::constants::{TREE_DEFAULT_OPEN_DEPTH, TreeExpanded};

pub(super) fn queue_tree_focus(
    commands: &mut ViewportCommandInbox,
    scene_index: &SceneAnchorIndex,
    entity: Entity,
    mode: FocusMode,
) {
    if let Some(target) = scene_index.anchor_for(entity) {
        commands.send(ViewportCommand::FocusTarget { target, mode });
    }
}

#[derive(Default, Clone)]
pub(super) struct RowOutcome {
    pub(super) clicked: Option<Entity>,
    pub(super) double_clicked: Option<Entity>,
    pub(super) ctx_action: Option<CtxAction>,
    pub(super) visibility_change: Option<(Entity, bool)>,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum CtxAction {
    FlyTo(Entity),
    Fit(Entity),
    ExpandDesc(Entity),
    CollapseDesc(Entity),
}

impl RowOutcome {
    pub(super) fn merge(&mut self, other: RowOutcome) {
        if other.double_clicked.is_some() {
            self.double_clicked = other.double_clicked;
        }
        if other.clicked.is_some() {
            self.clicked = other.clicked;
        }
        if other.ctx_action.is_some() {
            self.ctx_action = other.ctx_action;
        }
        if other.visibility_change.is_some() {
            self.visibility_change = other.visibility_change;
        }
    }
}

pub(super) fn set_subtree_expanded(
    root: Entity,
    prims: &Query<(Entity, &Name, &UsdPrimRef, Option<&UsdDisplayName>)>,
    children: &Query<&Children>,
    expanded: &mut TreeExpanded,
    open: bool,
) {
    let mut stack = vec![root];
    while let Some(e) = stack.pop() {
        if let Ok((_, _, pref, _)) = prims.get(e) {
            expanded.0.insert(pref.path.clone(), open);
        }
        if let Ok(cs) = children.get(e) {
            for c in cs.iter() {
                stack.push(c);
            }
        }
    }
}

pub(super) fn set_subtree_visible(
    root: Entity,
    children: &Query<&Children>,
    vis_cache: &mut HashMap<Entity, bool>,
    visible: bool,
) {
    let mut stack = vec![root];

    while let Some(entity) = stack.pop() {
        vis_cache.insert(entity, visible);

        if let Ok(entity_children) = children.get(entity) {
            for child in entity_children.iter() {
                stack.push(child);
            }
        }
    }
}

pub(super) fn swatch_color_for(
    entity: Entity,
    mat_q: &Query<&MeshMaterial3d<StandardMaterial>>,
    children: &Query<&Children>,
    materials: &Assets<StandardMaterial>,
) -> Option<egui::Color32> {
    let pick = |e: Entity| -> Option<egui::Color32> {
        let mm = mat_q.get(e).ok()?;
        let mat = materials.get(&mm.0)?;
        let c = mat.base_color.to_linear();
        Some(style::srgb_to_egui([c.red, c.green, c.blue]))
    };
    if let Some(c) = pick(entity) {
        return Some(c);
    }
    if let Ok(cs) = children.get(entity) {
        for c in cs.iter() {
            if let Some(col) = pick(c) {
                return Some(col);
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_tree_row(
    ui: &mut egui::Ui,
    entity: Entity,
    name: &Name,
    prim_ref: &UsdPrimRef,
    display_name: Option<&UsdDisplayName>,
    prims: &Query<(Entity, &Name, &UsdPrimRef, Option<&UsdDisplayName>)>,
    mat_q: &Query<&MeshMaterial3d<StandardMaterial>>,
    materials: &Assets<StandardMaterial>,
    vis_cache: &mut HashMap<Entity, bool>,
    children: &Query<&Children>,
    selected: &SelectedPrim,
    expanded: &mut TreeExpanded,
    accent: egui::Color32,
    depth: u32,
    leaf_override: bool,
    scroll_selected_to_top: bool,
) -> RowOutcome {
    let child_ids: Vec<Entity> = children
        .get(entity)
        .map(|c| c.iter().collect())
        .unwrap_or_default();
    let mut prim_children: Vec<(Entity, &Name, &UsdPrimRef, Option<&UsdDisplayName>)> = child_ids
        .iter()
        .filter_map(|c| prims.get(*c).ok())
        .collect();
    prim_children.sort_by(|a, b| a.2.path.cmp(&b.2.path));
    let has_prim_children = !prim_children.is_empty();
    let has_children = !leaf_override && has_prim_children;

    let is_selected = selected.0 == Some(entity);
    let path_key = prim_ref.path.clone();
    let row_id_salt = entity.to_bits();
    let mut outcome = RowOutcome::default();

    let mut visible_flag = *vis_cache.get(&entity).unwrap_or(&true);
    let visible_before = visible_flag;
    let swatch = swatch_color_for(entity, mat_q, children, materials);
    let mut color_sentinel = false;

    let label_owned: String = display_name
        .map(|d| d.0.clone())
        .unwrap_or_else(|| name.as_str().to_string());

    let resp = {
        let mut slot_buf: Vec<TreeIconSlot<'_>> = Vec::with_capacity(2);
        slot_buf.push(
            TreeIconSlot::new(TreeIconKind::Eye, &mut visible_flag)
                .with_tooltip("Toggle visibility"),
        );
        if let Some(c) = swatch {
            slot_buf.push(TreeIconSlot::new(
                TreeIconKind::Color(c),
                &mut color_sentinel,
            ));
        }

        if has_children {
            let is_open = *expanded
                .0
                .entry(path_key.clone())
                .or_insert(depth < TREE_DEFAULT_OPEN_DEPTH);
            let mut open_ref = is_open;
            let r = tree_row(
                ui,
                row_id_salt,
                depth,
                Some(&mut open_ref),
                None,
                &label_owned,
                is_selected,
                accent,
                &mut slot_buf,
            );
            if open_ref != is_open {
                expanded.0.insert(path_key.clone(), open_ref);
            }
            r
        } else {
            tree_row(
                ui,
                row_id_salt,
                depth,
                None,
                None,
                &label_owned,
                is_selected,
                accent,
                &mut slot_buf,
            )
        }
    };

    if scroll_selected_to_top && is_selected {
        resp.body.scroll_to_me(Some(egui::Align::TOP));
    }
    vis_cache.insert(entity, visible_flag);
    if has_prim_children && visible_flag != visible_before {
        set_subtree_visible(entity, children, vis_cache, visible_flag);
    }
    if visible_flag != visible_before {
        outcome.visibility_change = Some((entity, visible_flag));
    }
    if resp.body.hovered() {
        resp.body.clone().on_hover_text(&prim_ref.path);
    }
    if resp.body.double_clicked() {
        outcome.double_clicked = Some(entity);
    } else if resp.body.clicked() {
        outcome.clicked = Some(entity);
    }

    context_menu_frost(&resp.body, accent, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        if wide_button(ui, "Fly to", accent).clicked() {
            outcome.ctx_action = Some(CtxAction::FlyTo(entity));
            ui.close();
        }
        if wide_button(ui, "Fit to bounds", accent).clicked() {
            outcome.ctx_action = Some(CtxAction::Fit(entity));
            ui.close();
        }
        if wide_button(ui, "Copy path", accent).clicked() {
            ui.ctx().copy_text(prim_ref.path.clone());
            ui.close();
        }
        if wide_button(ui, "Expand descendants", accent).clicked() {
            outcome.ctx_action = Some(CtxAction::ExpandDesc(entity));
            ui.close();
        }
        if wide_button(ui, "Collapse descendants", accent).clicked() {
            outcome.ctx_action = Some(CtxAction::CollapseDesc(entity));
            ui.close();
        }
    });

    let show_children = if has_children {
        *expanded
            .0
            .get(&path_key)
            .unwrap_or(&(depth < TREE_DEFAULT_OPEN_DEPTH))
    } else {
        false
    };
    if show_children {
        for (child_entity, child_name, child_ref, child_dn) in prim_children {
            let sub = draw_tree_row(
                ui,
                child_entity,
                child_name,
                child_ref,
                child_dn,
                prims,
                mat_q,
                materials,
                vis_cache,
                children,
                selected,
                expanded,
                accent,
                depth + 1,
                false,
                scroll_selected_to_top,
            );
            outcome.merge(sub);
        }
    }

    outcome
}
