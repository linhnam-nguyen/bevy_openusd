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

mod constants;
mod panels;
mod plugin;
mod tree;

pub use constants::{
    RIB_INFO, RIB_KEYS, RIB_OVERLAYS, RIB_TREE, RIBBON_LEFT, TreeFilter, ViewerCommandPalette,
};
pub use plugin::ViewerUiPlugin;

pub(in crate::viewport::ui_frost) use constants::{
    PANEL_W, RIBBON_ITEMS, RIBBONS, TREE_DEFAULT_OPEN_DEPTH,
};
pub(in crate::viewport::ui_frost) use plugin::is_panel_open;
