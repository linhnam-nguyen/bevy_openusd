//! Viewer UI — bevy_frost ribbons + floating panels + widgets.
//!
//! Left rail is one `TwoSided` panel ribbon. Primary tools live in the
//! `Start` cluster (top-anchored); utility/help tools live in the `End`
//! cluster (bottom-anchored). Panel visibility is driven by the
//! `RibbonOpen` resource that frost ships with — clicking a rail button
//! toggles exclusively.
//!
//! PaneBuilder constraint: every pane body may ONLY call
//! `pane.section(id, title, default_open, body)`. Any free-standing
//! widget (sub_caption, readout_row, ScrollArea, …) must live inside
//! that body — which receives a regular `&mut egui::Ui`.

use bevy::asset::Assets;
use bevy::ecs::hierarchy::Children;
use bevy::mesh::Mesh3d;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use bevy_frost::prelude::*;
use bevy_frost::style;
use bevy_frost::widgets::section as nested_section;
use std::collections::HashMap;
use std::hash::Hash;
use std::path::PathBuf;
use usd_bevy::{UsdAsset, UsdDisplayName, UsdPrimRef, UsdProcedural, UsdSpatialAudio};

use crate::viewport::api::{SceneAnchorIndex, ViewportCommandInbox, ViewportReadModelState};
use crate::viewport::camera::ArcballCamera;
use crate::viewport::camera::{CameraBookmark, CameraBookmarks, CameraMount, FlyTo};
use crate::viewport::diagnostics::log_capture::{LoaderLog, LogLine};
use crate::viewport::scene::SelectedPrim;
use crate::viewport::session::{LoadRequest, LoaderTuning, StageInfo};
use viewport_protocol::{FocusMode, ViewportCommand};

mod tree;

// ─── Ribbon declaration ─────────────────────────────────────────────

pub const RIBBON_LEFT: &str = "viewer_left";

pub const RIB_SELECTION: &str = "viewer_selection";
pub const RIB_TREE: &str = "viewer_tree";
pub const RIB_INFO: &str = "viewer_info";
pub const RIB_VARIANTS: &str = "viewer_variants";
pub const RIB_CAMERAS: &str = "viewer_cameras";
pub const RIB_MATERIALS: &str = "viewer_materials";
pub const RIB_OVERLAYS: &str = "viewer_overlays";
pub const RIB_TIMELINE: &str = "viewer_timeline";
pub const RIB_KEYS: &str = "viewer_keys";
pub const RIB_LOG: &str = "viewer_log";
pub const RIB_PLAY: &str = "viewer_play";

const RIBBONS: &[RibbonDef] = &[RibbonDef {
    id: RIBBON_LEFT,
    edge: RibbonEdge::Left,
    role: RibbonRole::Panel,
    mode: RibbonMode::ThreeSided,
    draggable: false,
    accepts: &[],
}];

const RIBBON_ITEMS: &[RibbonItem] = &[
    RibbonItem {
        id: RIB_SELECTION,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 0,
        glyph: bevy_frost::RibbonGlyph::Text("F"),
        tooltip: "File / selection",
        child_ribbon: None,
    },
    RibbonItem {
        id: RIB_TREE,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 1,
        glyph: bevy_frost::RibbonGlyph::Text("T"),
        tooltip: "Prim tree (T)",
        child_ribbon: None,
    },
    RibbonItem {
        id: RIB_INFO,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 2,
        glyph: bevy_frost::RibbonGlyph::Text("i"),
        tooltip: "Stage info (I)",
        child_ribbon: None,
    },
    RibbonItem {
        id: RIB_VARIANTS,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 3,
        glyph: bevy_frost::RibbonGlyph::Text("V"),
        tooltip: "Variants",
        child_ribbon: None,
    },
    RibbonItem {
        id: RIB_CAMERAS,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 4,
        glyph: bevy_frost::RibbonGlyph::Text("C"),
        tooltip: "Cameras",
        child_ribbon: None,
    },
    RibbonItem {
        id: RIB_MATERIALS,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 5,
        glyph: bevy_frost::RibbonGlyph::Text("M"),
        tooltip: "Materials",
        child_ribbon: None,
    },
    RibbonItem {
        id: RIB_PLAY,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Middle,
        slot: 0,
        glyph: bevy_frost::RibbonGlyph::Text("▶"),
        tooltip: "Play / pause physics",
        child_ribbon: None,
    },
    RibbonItem {
        id: RIB_OVERLAYS,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::End,
        slot: 0,
        glyph: bevy_frost::RibbonGlyph::Text("O"),
        tooltip: "Overlays (O)",
        child_ribbon: None,
    },
    RibbonItem {
        id: RIB_TIMELINE,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::End,
        slot: 1,
        glyph: bevy_frost::RibbonGlyph::Text("⏱"),
        tooltip: "Timeline",
        child_ribbon: None,
    },
    RibbonItem {
        id: RIB_KEYS,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::End,
        slot: 2,
        glyph: bevy_frost::RibbonGlyph::Text("?"),
        tooltip: "Controls (?)",
        child_ribbon: None,
    },
    RibbonItem {
        id: RIB_LOG,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::End,
        slot: 3,
        glyph: bevy_frost::RibbonGlyph::Text("📜"),
        tooltip: "Log",
        child_ribbon: None,
    },
];

/// Legacy reference-tree expansion state. The delivered adapter uses the
/// renderer-neutral state in `tree::ProtocolTreeExpanded` instead.
///
/// This remains only so the pre-migration Frost implementation can be kept as
/// a source reference without being registered in the running UI.
#[derive(Resource, Default)]
#[allow(dead_code)]
pub struct TreeExpanded(pub HashMap<String, bool>);

/// Branches above this depth start expanded.
///
/// With roots at depth 0, a value of 2 displays roots, their direct
/// children, and second-level descendants without opening level 2.
const TREE_DEFAULT_OPEN_DEPTH: u32 = 2;

/// Free-text filter for the prim-tree panel. When non-empty, the
/// panel switches to a flat-list mode showing every prim whose path
/// contains the substring (case-insensitive).
#[derive(Resource, Default)]
pub struct TreeFilter(pub String);

/// Wrapper around frost's `CommandPaletteState` so Bevy can track it
/// as a Resource without needing to derive on an upstream type.
#[derive(Resource, Default)]
pub struct ViewerCommandPalette(pub CommandPaletteState);

/// The palette's static action list. Adding a new id here only
/// requires a matching arm in `dispatch_palette` below.
const PALETTE_ITEMS: &[PaletteItem] = &[
    PaletteItem {
        id: "open_selection",
        label: "Open: Selection panel",
        hint: Some("F"),
    },
    PaletteItem {
        id: "open_tree",
        label: "Open: Prim tree",
        hint: Some("T"),
    },
    PaletteItem {
        id: "open_info",
        label: "Open: Stage info",
        hint: Some("I"),
    },
    PaletteItem {
        id: "open_variants",
        label: "Open: Variants",
        hint: None,
    },
    PaletteItem {
        id: "open_cameras",
        label: "Open: Cameras",
        hint: None,
    },
    PaletteItem {
        id: "open_overlays",
        label: "Open: Overlays",
        hint: Some("O"),
    },
    PaletteItem {
        id: "open_timeline",
        label: "Open: Timeline",
        hint: None,
    },
    PaletteItem {
        id: "open_keys",
        label: "Open: Controls",
        hint: Some("?"),
    },
    PaletteItem {
        id: "open_log",
        label: "Open: Log",
        hint: None,
    },
    PaletteItem {
        id: "toggle_grid",
        label: "Toggle: Ground grid",
        hint: Some("G"),
    },
    PaletteItem {
        id: "toggle_axes",
        label: "Toggle: World axes",
        hint: Some("X"),
    },
    PaletteItem {
        id: "toggle_markers",
        label: "Toggle: Prim markers",
        hint: Some("P"),
    },
    PaletteItem {
        id: "toggle_wireframe",
        label: "Toggle: Wireframe",
        hint: None,
    },
    PaletteItem {
        id: "reload_stage",
        label: "Stage: Reload",
        hint: Some("R"),
    },
    PaletteItem {
        id: "browse_usd",
        label: "Stage: Browse for USD…",
        hint: None,
    },
];

// ─── Plugin ─────────────────────────────────────────────────────────

pub struct ViewerUiPlugin;

impl Plugin for ViewerUiPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<bevy_frost::FrostPlugin>() {
            app.add_plugins(bevy_frost::FrostPlugin);
        }
        app.init_resource::<TreeExpanded>()
            .init_resource::<TreeFilter>()
            .init_resource::<tree::ProtocolTreeExpanded>()
            .init_resource::<ViewerCommandPalette>()
            .add_systems(
                EguiPrimaryContextPass,
                (
                    draw_ribbons,
                    draw_selection_panel,
                    tree::draw_tree_panel,
                    draw_info_panel,
                    draw_variants_panel,
                    draw_cameras_panel,
                    draw_materials_panel,
                    draw_overlays_panel,
                    draw_timeline_panel,
                    draw_keys_panel,
                    draw_log_panel,
                    draw_palette_panel,
                )
                    .chain(),
            );
    }
}

const PANEL_W: f32 = 340.0;
const PANEL_H: f32 = 560.0;

// ─── Ribbon rail ────────────────────────────────────────────────────

/// Draws the activity ribbon and sends its physics action through the public
/// command path. Its active state is reduced from authoritative events.
fn draw_ribbons(
    mut contexts: EguiContexts,
    accent: Res<AccentColor>,
    mut open: ResMut<RibbonOpen>,
    mut placement: ResMut<RibbonPlacement>,
    mut drag: ResMut<RibbonDrag>,
    read_model: Res<ViewportReadModelState>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let physics_on = read_model
        .snapshot()
        .is_some_and(|snapshot| snapshot.physics_running);
    let clicks = draw_assembly(
        ctx,
        accent.0,
        RIBBONS,
        RIBBON_ITEMS,
        &mut open,
        &mut placement,
        &mut drag,
        |id| id == RIB_PLAY && physics_on,
    );
    for click in clicks {
        if click.item == RIB_PLAY {
            viewport_commands.send(ViewportCommand::SetPhysicsRunning {
                running: !physics_on,
            });
        }
    }
}

/// Tests whether an item in the viewer's left ribbon currently owns a panel.
fn is_panel_open(open: &RibbonOpen, item: &'static str) -> bool {
    open.is_open(RIBBON_LEFT, item)
}

// ─── Selection panel ────────────────────────────────────────────────

/// Draws the stage picker and details for the authoritative selected target.
fn draw_selection_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    requested: Res<crate::viewport::session::RequestedAsset>,
    mut load_req: ResMut<LoadRequest>,
    read_model: Res<ViewportReadModelState>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
) {
    if !is_panel_open(&open, RIB_SELECTION) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let stage_name = read_model
        .snapshot()
        .map(|snapshot| snapshot.stage.display_name.clone())
        .unwrap_or_else(|| "(stage loading)".to_owned());
    let selection = read_model
        .snapshot()
        .and_then(|snapshot| snapshot.selection.target.clone());
    let accent_col = accent.0;
    let mut keep = true;
    floating_window_for_item(
        ctx,
        RIBBONS,
        RIBBON_ITEMS,
        &placement,
        RIB_SELECTION,
        "Selection",
        egui::vec2(PANEL_W, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("sel_stage", "Loaded stage", true, |ui| {
                readout_row(ui, "file", &stage_name);
                if wide_button(ui, "📁  Browse USD…", accent_col).clicked()
                    && let Some(picked) = rfd::FileDialog::new()
                        .add_filter("USD stages", &["usda", "usdc", "usd", "usdz"])
                        .pick_file()
                {
                    load_req.path = Some(PathBuf::from(picked));
                }
                if wide_button(ui, "🗂  Reveal in filesystem", accent_col).clicked() {
                    let full = requested.root.join(&stage_name);
                    let target = full.parent().unwrap_or(&requested.root).to_path_buf();
                    let _ = std::process::Command::new("xdg-open").arg(&target).spawn();
                }
            });
            pane.section("sel_prim", "Selected prim", true, |ui| match &selection {
                Some(target) => {
                    readout_row(
                        ui,
                        "name",
                        target
                            .prim_path
                            .rsplit('/')
                            .next()
                            .unwrap_or(&target.prim_path),
                    );
                    readout_row(ui, "path", &target.prim_path);
                    if let Some(context) = &target.instance_context {
                        readout_row(ui, "instance", context);
                    }
                    if wide_button(ui, "Clear selection", accent_col).clicked() {
                        viewport_commands.send(ViewportCommand::SelectTarget { target: None });
                    }
                }
                None => sub_caption(ui, "Click a prim in the Tree panel"),
            });
        },
    );
}

// ─── Prim-tree panel ────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
/// Draws the searchable USD prim hierarchy and applies row interactions.
fn draw_legacy_tree_panel(
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
    // Combined with `Option<&UsdDisplayName>` so the system stays
    // under Bevy's 16-SystemParam limit. The recursive row helper
    // pulls the display name via `prims.get(entity)` instead of
    // a separate query.
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

                // Snapshot current visibility so rows can be rendered with
                // local booleans. The final subtree action is sent through
                // the viewport contract below, not written to ECS here.
                let mut vis_cache: HashMap<Entity, bool> = HashMap::new();
                for (e, v) in visibility_q.iter() {
                    vis_cache.insert(e, !matches!(*v, Visibility::Hidden));
                }

                let filter_lc = filter.0.to_lowercase();
                let flat = !filter_lc.is_empty();

                let mut outcome = RowOutcome::default();
                // Hardcoded generous viewport — frost's `section`
                // allocates the body Ui with initial height 0, so
                // `available_height` here would clip the scroll list
                // to almost nothing. 600 px gives ~30 visible rows
                // at the default `TREE_ROW_H = 20`; the panel itself
                // opens 720 px tall so this fits without overflow.
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

/// Converts a Frost tree-row entity into the same logical command sent by a
/// future product UI. The scene index owns the private entity mapping, so the
/// command never exposes an ECS identifier.
#[allow(dead_code)]
fn queue_tree_focus(
    commands: &mut ViewportCommandInbox,
    scene_index: &SceneAnchorIndex,
    entity: Entity,
    mode: FocusMode,
) {
    if let Some(target) = scene_index.anchor_for(entity) {
        commands.send(ViewportCommand::FocusTarget { target, mode });
    }
}

#[allow(dead_code)]
#[derive(Default, Clone)]
struct RowOutcome {
    clicked: Option<Entity>,
    double_clicked: Option<Entity>,
    ctx_action: Option<CtxAction>,
    visibility_change: Option<(Entity, bool)>,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
enum CtxAction {
    FlyTo(Entity),
    Fit(Entity),
    ExpandDesc(Entity),
    CollapseDesc(Entity),
}

#[allow(dead_code)]
impl RowOutcome {
    /// Folds a child row's latest interaction into this subtree result.
    fn merge(&mut self, other: RowOutcome) {
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

/// Walk the subtree rooted at `root` and set each descendant's
/// `TreeExpanded` entry to `open`. Used by the row context-menu
/// "Expand / Collapse descendants" actions.
#[allow(dead_code)]
fn set_subtree_expanded(
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

/// Sets the cached visibility state for `root` and all its descendants.
#[allow(dead_code)]
fn set_subtree_visible(
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

/// Lookup the first-bound material's `base_color` for `entity` (or
/// one of its direct mesh-carrying children) and convert linear sRGB
/// into an egui colour suitable for a tree-row swatch.
#[allow(dead_code)]
fn swatch_color_for(
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
#[allow(dead_code)]
/// Recursively renders one prim row and returns its latest user interaction.
fn draw_tree_row(
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
    // Force a leaf-style row (no chevron, no descendants). Used by
    // the flat filter mode where we render ancestorless hits.
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
    // Tree-row egui id: entity's bits, NOT the prim path. Production
    // assets (Davinci, PointInstancer expansion, internal-reference
    // dedup) routinely produce multiple entities sharing one prim
    // path; using the path as id_salt collides those rows in egui's
    // internal id arena and blasts the console with "ID is not
    // unique" warnings. Entity IDs are guaranteed unique within the
    // ECS world — perfect.
    let row_id_salt = entity.to_bits();
    let mut outcome = RowOutcome::default();

    // Eye + swatch slots.
    let mut visible_flag = *vis_cache.get(&entity).unwrap_or(&true);
    let visible_before = visible_flag;
    let swatch = swatch_color_for(entity, mat_q, children, materials);
    let mut color_sentinel = false;

    // Label preference: authored `ui:displayName` (UsdUI) > prim leaf
    // name. Most stages won't author a display name and fall straight
    // through to the leaf.
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

    // Write the eye state back to the cache; the panel commits it
    // to the ECS after all rows have rendered.
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

// ─── Stage-info panel ───────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
/// Draws the loaded-stage metadata and projection summary panel.
fn draw_info_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    info: Res<StageInfo>,
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

// ─── Variants panel ─────────────────────────────────────────────────

/// Draws variant-set controls and records pending reload selections.
fn draw_variants_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    usd_assets: Res<Assets<UsdAsset>>,
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
                let asset = usd_assets.iter().next().map(|(_, a)| a);
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
                    if asset.skel_animations.is_empty() {
                        sub_caption(ui, "No UsdSkel animations or `anim` variant set found.");
                    } else {
                        sub_caption(
                            ui,
                            &format!(
                                "{} SkelAnimation prim(s) found; this stage does not expose an `anim` variant switch.",
                                asset.skel_animations.len()
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
                let asset = usd_assets.iter().next().map(|(_, a)| a);
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

/// Local dropdown for long USD variant / animation lists. Frost's stock
/// dropdown paints every option directly in the popup, so the cow's many
/// `anim` clips can run off-screen. egui's ComboBox has a built-in scroll
/// area via `.height(...)`, while still returning a normal changed Response.
/// Renders a scrollable option control while preserving the selected value.
fn scroll_dropdown_control(
    ui: &mut egui::Ui,
    id_salt: impl Hash + std::fmt::Debug,
    selected: &mut usize,
    options: &[&str],
    _accent: egui::Color32,
) -> egui::Response {
    let display = options.get(*selected).copied().unwrap_or("—");
    let max_w = ui.available_width().max(60.0).min(200.0);
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
                    // Keep focus on the selector while using Up/Down to
                    // scrub through clips. Without this, egui treats the
                    // first arrow press as focus navigation and moves focus
                    // away after a single selection change.
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

// ─── Cameras panel ──────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
/// Lists authored cameras and manages mounting, bookmarks, and camera navigation.
fn draw_cameras_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    usd_assets: Res<Assets<UsdAsset>>,
    mut camera_mount: ResMut<CameraMount>,
    mut bookmarks: ResMut<CameraBookmarks>,
    mut fly: ResMut<FlyTo>,
    cameras: Query<&ArcballCamera>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
) {
    if !is_panel_open(&open, RIB_CAMERAS) {
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
        RIB_CAMERAS,
        "Cameras",
        egui::vec2(PANEL_W, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("cameras_bookmarks", "Bookmarks", true, |ui| {
                if wide_button(ui, "💾  Save current view", accent_col).clicked() {
                    if let Ok(cam) = cameras.single() {
                        let seq = bookmarks.next_seq + 1;
                        bookmarks.next_seq = seq;
                        bookmarks.items.push(CameraBookmark {
                            name: format!("View {seq}"),
                            focus: cam.focus,
                            distance: cam.distance,
                            yaw: cam.yaw,
                            elevation: cam.elevation,
                        });
                    }
                }
                if bookmarks.items.is_empty() {
                    sub_caption(ui, "(no bookmarks yet)");
                } else {
                    let mut to_delete: Option<usize> = None;
                    let mut to_jump: Option<usize> = None;
                    for (idx, bm) in bookmarks.items.iter().enumerate() {
                        let r = hybrid_select_row(
                            ui,
                            ("bookmark", idx),
                            &bm.name,
                            Some(&format!("d {:.1}", bm.distance)),
                            false,
                            false,
                            accent_col,
                        );
                        if r.body.clicked() {
                            to_jump = Some(idx);
                        }
                        if r.radio.clicked() {
                            to_delete = Some(idx);
                        }
                    }
                    if let Some(idx) = to_jump
                        && let (Ok(cam), Some(bm)) = (cameras.single(), bookmarks.items.get(idx))
                    {
                        *camera_mount = CameraMount::Arcball;
                        fly.start_focus = cam.focus;
                        fly.start_distance = cam.distance;
                        fly.start_yaw = Some(cam.yaw);
                        fly.start_elevation = Some(cam.elevation);
                        fly.target_focus = bm.focus;
                        fly.target_distance = bm.distance;
                        fly.target_yaw = Some(bm.yaw);
                        fly.target_elevation = Some(bm.elevation);
                        fly.duration = 0.5;
                        fly.remaining = 0.5;
                    }
                    if let Some(idx) = to_delete {
                        bookmarks.items.remove(idx);
                    }
                    sub_caption(ui, "Click row to jump · click radio to delete");
                }
            });

            pane.section("cameras_all", "Cameras", true, |ui| {
                let asset = usd_assets.iter().next().map(|(_, a)| a);
                let Some(asset) = asset else {
                    sub_caption(ui, "(no stage loaded yet)");
                    return;
                };
                sub_caption(ui, &format!("{} authored cameras", asset.cameras.len()));
                ui.add_space(style::space::BLOCK);

                let arcball_active = matches!(*camera_mount, CameraMount::Arcball);
                let r = hybrid_select_row(
                    ui,
                    "arcball_mount",
                    "🎮  Arcball (free)",
                    None,
                    arcball_active,
                    arcball_active,
                    accent_col,
                );
                if r.body.clicked() || r.radio.clicked() {
                    viewport_commands.send(ViewportCommand::SetCameraSource {
                        source: viewport_protocol::CameraSource::Arcball,
                    });
                }

                row_separator(ui);

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for cam in &asset.cameras {
                        let mounted = matches!(
                            &*camera_mount,
                            CameraMount::Mounted { prim_path } if prim_path == &cam.path
                        );
                        let name = cam.path.rsplit('/').next().unwrap_or(&cam.path);
                        let focal = cam.data.focal_length_mm.unwrap_or(50.0);
                        let proj = match cam.data.projection {
                            Some(usd_schema::camera::Projection::Orthographic) => "ortho",
                            _ => "persp",
                        };
                        let label = format!("📷  {name}");
                        let trailing = format!("{focal:.0}mm · {proj}");
                        let r = hybrid_select_row(
                            ui,
                            cam.path.as_str(),
                            &label,
                            Some(&trailing),
                            mounted,
                            mounted,
                            accent_col,
                        );
                        if r.body.clicked() || r.radio.clicked() {
                            viewport_commands.send(ViewportCommand::SetCameraSource {
                                source: viewport_protocol::CameraSource::Authored {
                                    prim_path: cam.path.clone(),
                                },
                            });
                        }
                    }
                });
            });
        },
    );
}

// ─── Materials panel ────────────────────────────────────────────────
//
// Lets the user override per-material `StandardMaterial` properties
// at runtime. Useful when an asset author shipped placeholder colours
// instead of textures (Scout V2 with yellow wheels, Jackal with
// `material_yellow` strips, …) — mutating the underlying asset
// propagates the new colour to every mesh that bound this material,
// no per-mesh override needed.

/// Shows material bindings and material properties for the current stage.
fn draw_materials_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    // Only entities tagged with `UsdPrimRef` came from the loaded
    // USD asset — restrict the panel to those so it doesn't list
    // glacial's grid materials, the gizmo lines, the ground floor,
    // or any other internal viewer geometry.
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

    // Stable presentation order: by asset path / id.
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
                        // Base colour. Bevy's StandardMaterial.base_color
                        // is in linear sRGB; egui's color picker thinks
                        // gamma-corrected sRGB. Round-trip through linear
                        // so what the user sees in the picker matches
                        // what gets stored.
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

// ─── Overlays panel ─────────────────────────────────────────────────

/// Sends a Frost toggle through the same presentation command used by a host UI.
fn protocol_overlay_toggle(
    ui: &mut egui::Ui,
    label: &str,
    current: bool,
    overlay: viewport_protocol::OverlayKind,
    accent: egui::Color32,
    commands: &mut ViewportCommandInbox,
) {
    let mut value = current;
    if toggle(ui, label, &mut value, accent).changed() {
        commands.send(ViewportCommand::SetOverlay {
            overlay,
            enabled: value,
        });
    }
}

/// Exposes debug-overlay, wireframe, lighting, and collider visibility controls.
fn draw_overlays_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    read_model: Res<ViewportReadModelState>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
) {
    if !is_panel_open(&open, RIB_OVERLAYS) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let Some(snapshot) = read_model.snapshot() else {
        return;
    };
    let presentation = &snapshot.presentation;
    let accent_col = accent.0;
    let mut keep = true;
    let mut curve_tuning = presentation.curve_tuning;
    floating_window_for_item(
        ctx,
        RIBBONS,
        RIBBON_ITEMS,
        &placement,
        RIB_OVERLAYS,
        "Overlays",
        egui::vec2(PANEL_W, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("overlay_toggles", "World overlays", true, |ui| {
                protocol_overlay_toggle(
                    ui,
                    "Ground grid (G)",
                    presentation.ground_grid,
                    viewport_protocol::OverlayKind::GroundGrid,
                    accent_col,
                    &mut viewport_commands,
                );
                protocol_overlay_toggle(
                    ui,
                    "World axes (X)",
                    presentation.world_axes,
                    viewport_protocol::OverlayKind::WorldAxes,
                    accent_col,
                    &mut viewport_commands,
                );
                protocol_overlay_toggle(
                    ui,
                    "Prim markers (P)",
                    presentation.prim_markers,
                    viewport_protocol::OverlayKind::PrimMarkers,
                    accent_col,
                    &mut viewport_commands,
                );
                let mut v = presentation.prim_marker_bias as f64;
                if pretty_slider(
                    ui,
                    "Prim marker bias",
                    &mut v,
                    0.0..=5.0,
                    2,
                    "×",
                    accent_col,
                )
                .changed()
                {
                    viewport_commands.send(ViewportCommand::SetPrimMarkerBias { bias: v as f32 });
                }
                protocol_overlay_toggle(
                    ui,
                    "Skeleton bones (B)",
                    presentation.skeleton,
                    viewport_protocol::OverlayKind::Skeleton,
                    accent_col,
                    &mut viewport_commands,
                );
                protocol_overlay_toggle(
                    ui,
                    "Physics gizmos (Y)",
                    presentation.physics,
                    viewport_protocol::OverlayKind::Physics,
                    accent_col,
                    &mut viewport_commands,
                );
                protocol_overlay_toggle(
                    ui,
                    "Collider wireframes (C)",
                    presentation.colliders,
                    viewport_protocol::OverlayKind::Colliders,
                    accent_col,
                    &mut viewport_commands,
                );
            });

            pane.section("overlay_render", "Render", true, |ui| {
                protocol_overlay_toggle(
                    ui,
                    "Wireframe",
                    presentation.wireframe,
                    viewport_protocol::OverlayKind::Wireframe,
                    accent_col,
                    &mut viewport_commands,
                );
                let mut s = presentation.light_intensity_scale as f64;
                if pretty_slider(ui, "Light intensity", &mut s, 0.0..=5.0, 2, "×", accent_col)
                    .changed()
                {
                    viewport_commands.send(ViewportCommand::SetLightIntensity { scale: s as f32 });
                }
                sub_caption(ui, "Scales every authored light from its original value.");
            });

            pane.section("overlay_curves", "Curves (tubes)", true, |ui| {
                sub_caption(ui, "Default radius used when widths aren't authored");
                let mut r = curve_tuning.default_radius as f64;
                if pretty_slider(ui, "Radius", &mut r, 0.001..=0.2, 3, " m", accent_col).changed() {
                    curve_tuning.default_radius = r as f32;
                    viewport_commands.send(ViewportCommand::SetCurveTuning {
                        tuning: curve_tuning,
                    });
                }
                let mut seg = curve_tuning.ring_segments as f64;
                if pretty_slider(ui, "Ring segments", &mut seg, 3.0..=24.0, 0, "", accent_col)
                    .changed()
                {
                    curve_tuning.ring_segments = seg.round() as u32;
                    viewport_commands.send(ViewportCommand::SetCurveTuning {
                        tuning: curve_tuning,
                    });
                }
                let mut ps = curve_tuning.point_scale as f64;
                if pretty_slider(ui, "Point scale", &mut ps, 0.05..=4.0, 2, "×", accent_col)
                    .changed()
                {
                    curve_tuning.point_scale = ps as f32;
                    viewport_commands.send(ViewportCommand::SetCurveTuning {
                        tuning: curve_tuning,
                    });
                }
                sub_caption(ui, "Sliders apply live — no reload needed.");
            });
        },
    );
}

// ─── Timeline panel ─────────────────────────────────────────────────

/// Draws playback, scrub, and animation-clip controls for USD time samples.
fn draw_timeline_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    read_model: Res<ViewportReadModelState>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
    usd_assets: Res<Assets<UsdAsset>>,
) {
    if !is_panel_open(&open, RIB_TIMELINE) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let Some(snapshot) = read_model.snapshot() else {
        return;
    };
    let timeline = &snapshot.timeline;
    let duration_seconds = (timeline.end_time_code - timeline.start_time_code).max(0.0)
        / timeline.time_codes_per_second.max(f64::MIN_POSITIVE);
    let accent_col = accent.0;
    let mut keep = true;
    floating_window_for_item(
        ctx,
        RIBBONS,
        RIBBON_ITEMS,
        &placement,
        RIB_TIMELINE,
        "Timeline",
        egui::vec2(PANEL_W, 320.0),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("timeline_playback", "Playback", true, |ui| {
                let asset = usd_assets.iter().next().map(|(_, a)| a);
                let animated_count = asset.map(|a| a.animated_prims.len()).unwrap_or(0);
                sub_caption(
                    ui,
                    &format!(
                        "{animated_count} animated prim(s) · {:.1} fps · {:.1}s total",
                        timeline.time_codes_per_second, duration_seconds
                    ),
                );
                ui.add_space(style::space::BLOCK);

                let play_label = if timeline.playing {
                    "⏸  Pause"
                } else {
                    "▶  Play"
                };
                if wide_button(ui, play_label, accent_col).clicked() {
                    viewport_commands.send(ViewportCommand::SetPlayback {
                        playing: !timeline.playing,
                    });
                }
                if wide_button(ui, "⏮  Rewind", accent_col).clicked() {
                    viewport_commands.send(ViewportCommand::Seek { seconds: 0.0 });
                }

                ui.add_space(style::space::BLOCK);
                let dur = duration_seconds.max(1e-3);
                let mut seconds = timeline.seconds;
                if pretty_slider(ui, "Seconds", &mut seconds, 0.0..=dur, 3, " s", accent_col)
                    .changed()
                {
                    viewport_commands.send(ViewportCommand::Seek { seconds });
                }

                readout_row(
                    ui,
                    "timeCode",
                    &format!(
                        "{:.3}",
                        timeline.start_time_code
                            + timeline.seconds * timeline.time_codes_per_second
                    ),
                );
                readout_row(
                    ui,
                    "range",
                    &format!(
                        "{:.2} … {:.2}",
                        timeline.start_time_code, timeline.end_time_code
                    ),
                );
                readout_row(ui, "fps", &format!("{:.2}", timeline.time_codes_per_second));
            });
        },
    );
}

// ─── Keys panel ─────────────────────────────────────────────────────

/// Displays the viewer's keyboard and mouse interaction reference.
fn draw_keys_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
) {
    if !is_panel_open(&open, RIB_KEYS) {
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
        RIB_KEYS,
        "Controls",
        egui::vec2(PANEL_W, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("keys_camera", "Camera", true, |ui| {
                keybinding_row(ui, "L+R drag", "Orbit");
                keybinding_row(ui, "Middle", "Pan");
                keybinding_row(ui, "Scroll", "Zoom");
            });
            pane.section("keys_panels", "Panels", true, |ui| {
                keybinding_row(ui, "T", "Toggle prim tree");
                keybinding_row(ui, "I", "Toggle stage info");
                keybinding_row(ui, "O", "Toggle overlays");
                keybinding_row(ui, "?", "Toggle this panel");
            });
            pane.section("keys_overlays", "Overlays", true, |ui| {
                keybinding_row(ui, "G", "Ground grid");
                keybinding_row(ui, "X", "World axes");
                keybinding_row(ui, "P", "Prim markers");
                keybinding_row(ui, "B", "Skeleton bones");
            });
            pane.section("keys_stage", "Stage", true, |ui| {
                keybinding_row(ui, "R", "Reload stage from disk");
            });
        },
    );
    // Suppress unused warning when accent_col isn't read inside bodies.
    let _ = accent_col;
}

// ─── Log panel ──────────────────────────────────────────────────────

/// Displays the in-app log buffer with level filtering and target shortening.
fn draw_log_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    log: Res<LoaderLog>,
) {
    if !is_panel_open(&open, RIB_LOG) {
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
        RIB_LOG,
        "Log",
        egui::vec2(PANEL_W + 80.0, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("log_lines", "Loader log", true, |ui| {
                let count = log.buffer.lock().map(|b| b.len()).unwrap_or(0);
                sub_caption(ui, &format!("{count} entries · capped at 500"));
                ui.horizontal(|ui| {
                    if ui.small_button("Clear").clicked()
                        && let Ok(mut buf) = log.buffer.lock()
                    {
                        buf.clear();
                    }
                });
                ui.add_space(style::space::TIGHT);

                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        let snapshot: Vec<LogLine> = log
                            .buffer
                            .lock()
                            .map(|b| b.iter().cloned().collect())
                            .unwrap_or_default();
                        if snapshot.is_empty() {
                            sub_caption(ui, "(no events yet — load a stage)");
                            return;
                        }
                        for line in &snapshot {
                            let level_color = level_to_color(line.level);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                ui.painter().rect_filled(
                                    egui::Rect::from_center_size(
                                        ui.cursor().min + egui::vec2(4.0, 8.0),
                                        egui::vec2(6.0, 6.0),
                                    ),
                                    egui::CornerRadius::same(1),
                                    level_color,
                                );
                                ui.add_space(10.0);
                                ui.label(
                                    egui::RichText::new(short_target(&line.target))
                                        .small()
                                        .monospace()
                                        .color(style::TEXT_SECONDARY),
                                );
                                ui.label(
                                    egui::RichText::new(&line.message)
                                        .small()
                                        .color(style::TEXT_PRIMARY),
                                );
                            });
                        }
                    });
            });
        },
    );
}

/// Maps a Bevy log severity to the panel's readable foreground colour.
fn level_to_color(level: bevy::log::Level) -> egui::Color32 {
    match level {
        bevy::log::Level::ERROR => style::DANGER,
        bevy::log::Level::WARN => style::WARNING,
        bevy::log::Level::INFO => style::SUCCESS,
        _ => style::TEXT_SECONDARY,
    }
}

/// Condenses a Rust module path for narrow log-panel rows.
fn short_target(target: &str) -> String {
    // `usd_bevy::asset` → `asset`. Drops the crate prefix so the
    // log row stays readable at panel width.
    target.rsplit("::").next().unwrap_or(target).to_string()
}

// ─── Command palette (Ctrl+K) ───────────────────────────────────────

#[allow(clippy::too_many_arguments)]
/// Draws the command palette and dispatches the selected action.
fn draw_palette_panel(
    mut contexts: EguiContexts,
    accent: Res<AccentColor>,
    mut palette: ResMut<ViewerCommandPalette>,
    mut ribbon: ResMut<RibbonOpen>,
    read_model: Res<ViewportReadModelState>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
    mut load_req: ResMut<LoadRequest>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let Some(id) = command_palette(ctx, &mut palette.0, PALETTE_ITEMS, accent.0) else {
        return;
    };
    match id {
        "open_selection" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_SELECTION);
        }
        "open_tree" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_TREE);
        }
        "open_info" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_INFO);
        }
        "open_variants" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_VARIANTS);
        }
        "open_cameras" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_CAMERAS);
        }
        "open_overlays" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_OVERLAYS);
        }
        "open_timeline" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_TIMELINE);
        }
        "open_keys" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_KEYS);
        }
        "open_log" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_LOG);
        }
        "toggle_grid" => {
            if let Some(snapshot) = read_model.snapshot() {
                viewport_commands.send(ViewportCommand::SetOverlay {
                    overlay: viewport_protocol::OverlayKind::GroundGrid,
                    enabled: !snapshot.presentation.ground_grid,
                });
            }
        }
        "toggle_axes" => {
            if let Some(snapshot) = read_model.snapshot() {
                viewport_commands.send(ViewportCommand::SetOverlay {
                    overlay: viewport_protocol::OverlayKind::WorldAxes,
                    enabled: !snapshot.presentation.world_axes,
                });
            }
        }
        "toggle_markers" => {
            if let Some(snapshot) = read_model.snapshot() {
                viewport_commands.send(ViewportCommand::SetOverlay {
                    overlay: viewport_protocol::OverlayKind::PrimMarkers,
                    enabled: !snapshot.presentation.prim_markers,
                });
            }
        }
        "toggle_wireframe" => {
            if let Some(snapshot) = read_model.snapshot() {
                viewport_commands.send(ViewportCommand::SetOverlay {
                    overlay: viewport_protocol::OverlayKind::Wireframe,
                    enabled: !snapshot.presentation.wireframe,
                });
            }
        }
        "reload_stage" => {
            viewport_commands.send(ViewportCommand::ReloadSession);
        }
        "browse_usd" => {
            if let Some(picked) = rfd::FileDialog::new()
                .add_filter("USD stages", &["usda", "usdc", "usd", "usdz"])
                .pick_file()
            {
                load_req.path = Some(PathBuf::from(picked));
            }
        }
        _ => {}
    }
    palette.0.open = false;
}
