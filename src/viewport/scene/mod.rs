//! Scene-derived state and visual presentation controls.

mod classification_color;
mod diff;
mod ghost;
mod section_box;
mod section_box_clipping;
mod section_box_gizmo;
mod section_box_visualization;
mod selection;
mod selection_color;
mod selection_hover;
mod selection_outline;
mod selection_projection;
mod skeleton;
mod solari;

pub(in crate::viewport) use classification_color::{
    ClassificationColorDiagnostics, ClassificationColorMaterialCache, ClassificationColorPlan,
    sync_classification_color_overrides,
};
pub(crate) use diff::draw_semantic_diff;
pub(crate) use ghost::{HistoricalGhostState, hydrate_historical_ghosts};
#[allow(unused_imports)]
pub(in crate::viewport) use section_box::{
    SectionBoxState, aggregate_selection_bounds, selected_renderable_entities,
    sync_section_box_state,
};
pub(in crate::viewport) use section_box_clipping::{
    SectionClipMaterial, sync_section_box_clipping,
};
#[allow(unused_imports)]
pub(in crate::viewport) use section_box_gizmo::{
    SectionBoxGizmoTarget, capture_section_box_gizmo_transform, sync_section_box_gizmo_target,
};
pub(in crate::viewport) use section_box_visualization::draw_section_box;
pub(crate) use selection::{SelectedPrim, SelectedTargets, sync_selected_instance_identity};
#[allow(unused_imports)]
pub(in crate::viewport) use selection_color::{
    HoverColorMaterial, SelectionBaseMaterial, SelectionColorMaterial, SelectionColorOverride,
    SelectionColorOverrideState, sync_selection_color_overrides,
};
#[allow(unused_imports)]
pub(in crate::viewport) use selection_hover::HoveredTarget;
#[allow(unused_imports)]
pub(in crate::viewport) use selection_outline::SelectionOutline;
pub(in crate::viewport) use selection_outline::{SelectionOutlineState, sync_selection_outlines};
pub(in crate::viewport) use selection_projection::{
    SelectedRenderableProjection, sync_selected_renderable_projection,
};
pub(crate) use skeleton::{
    HideMeshesFlag, ShowJointGizmosFlag, SkeletonGizmos, hide_meshes_on_startup,
    setup_skeleton_gizmos_on_top,
};
pub(crate) use solari::{SolariCapability, SolariCapabilityPlugin};
pub(crate) mod extent;
pub(crate) mod visualization;
pub use extent::SceneExtent;
