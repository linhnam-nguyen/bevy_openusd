//! Scene-derived state and visual presentation controls.

mod diff;
mod ghost;
mod section_box;
mod section_box_visualization;
mod selection;
mod selection_color;
mod selection_hover;
mod selection_outline;
mod skeleton;
mod solari;

pub(crate) use diff::draw_semantic_diff;
pub(crate) use ghost::{HistoricalGhostState, hydrate_historical_ghosts};
pub(in crate::viewport) use section_box::{SectionBoxState, sync_section_box_state};
pub(in crate::viewport) use section_box_visualization::draw_section_box;
pub(crate) use selection::{SelectedPrim, SelectedTargets, sync_selected_instance_identity};
pub(in crate::viewport) use selection_outline::{SelectionOutlineState, sync_selection_outlines};
pub(crate) use skeleton::{
    HideMeshesFlag, ShowJointGizmosFlag, SkeletonGizmos, hide_meshes_on_startup,
    setup_skeleton_gizmos_on_top,
};
pub(crate) use solari::{SolariCapability, SolariCapabilityPlugin};
pub(crate) mod extent;
pub(crate) mod visualization;
pub use extent::SceneExtent;
