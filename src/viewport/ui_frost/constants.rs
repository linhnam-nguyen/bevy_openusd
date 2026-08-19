use bevy::prelude::Resource;
use bevy_frost::prelude::*;
use std::collections::HashMap;

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

pub(in crate::viewport::ui_frost) const RIBBONS: &[RibbonDef] = &[RibbonDef {
    id: RIBBON_LEFT,
    edge: RibbonEdge::Left,
    role: RibbonRole::Panel,
    mode: RibbonMode::ThreeSided,
    draggable: false,
    accepts: &[],
}];

pub(in crate::viewport::ui_frost) const RIBBON_ITEMS: &[RibbonItem] = &[
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

/// Legacy reference-tree expansion state.
#[derive(Resource, Default)]
#[allow(dead_code)]
pub struct TreeExpanded(pub HashMap<String, bool>);

/// Branches above this depth start expanded.
#[allow(dead_code)]
pub(in crate::viewport::ui_frost) const TREE_DEFAULT_OPEN_DEPTH: u32 = 2;

/// Free-text filter for the prim-tree panel.
#[derive(Resource, Default)]
pub struct TreeFilter(pub String);

/// Wrapper around frost's `CommandPaletteState`.
#[derive(Resource, Default)]
pub struct ViewerCommandPalette(pub CommandPaletteState);

/// The palette's static action list.
pub(in crate::viewport::ui_frost) const PALETTE_ITEMS: &[PaletteItem] = &[
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

pub(in crate::viewport::ui_frost) const PANEL_W: f32 = 340.0;
pub(in crate::viewport::ui_frost) const PANEL_H: f32 = 560.0;
