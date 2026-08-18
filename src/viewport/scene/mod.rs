//! Scene-derived state and visual presentation controls.

mod diff;
mod runtime;
mod selection;
mod skeleton;

pub(crate) use diff::draw_semantic_diff;
pub(crate) use runtime::draw_selected_prim_highlight;
pub(crate) use selection::SelectedPrim;
pub(crate) use skeleton::{
    HideMeshesFlag, ShowJointGizmosFlag, SkeletonGizmos, hide_meshes_on_startup,
    setup_skeleton_gizmos_on_top,
};
pub(crate) mod visualization;
