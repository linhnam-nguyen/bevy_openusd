//! Scene-derived state and visual presentation controls.

mod diff;
mod ghost;
mod selection;
mod selection_color;
mod selection_outline;
mod skeleton;
mod solari;

pub(crate) use diff::draw_semantic_diff;
pub(crate) use ghost::{HistoricalGhostState, hydrate_historical_ghosts};
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
