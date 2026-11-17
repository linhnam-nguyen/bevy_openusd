//! Scene-derived state and visual presentation controls.

mod classification_color;
mod diff;
mod entity_order;
mod ghost;
mod projection_work;
mod section_box;
mod selection;
mod skeleton;
mod solari;

pub(in crate::viewport) use classification_color::{
    ClassificationColorDiagnostics, ClassificationColorMaterialCache, ClassificationColorPlan,
    refresh_classification_color_plan, sync_classification_color_overrides,
};
pub(crate) use diff::draw_semantic_diff;
pub(crate) use ghost::{HistoricalGhostState, hydrate_historical_ghosts};
#[allow(unused_imports)]
pub(in crate::viewport) use section_box::{
    SectionBoxGizmoTarget, SectionBoxState, SectionClipMaterial, aggregate_selection_bounds,
    capture_section_box_gizmo_transform, draw_section_box, selected_renderable_entities,
    sync_section_box_clipping, sync_section_box_gizmo_target, sync_section_box_state,
};
#[allow(unused_imports)]
pub(in crate::viewport) use selection::{
    CoarseSelectionPresentation, CoarseSelectionProxyState, HoverColorMaterial, HoveredTarget,
    ProjectedWorldBounds, SelectedRenderableProjection, SelectionBaseMaterial,
    SelectionColorMaterial, SelectionColorOverride, SelectionColorOverrideState, SelectionOutline,
    SelectionOutlineState, SelectionPresentationPolicy, draw_coarse_selection_proxy,
    register_selection_projection_observers, sync_coarse_selection_proxy,
    sync_selected_renderable_projection, sync_selection_color_overrides, sync_selection_outlines,
};
pub(crate) use selection::{SelectedPrim, SelectedTargets, sync_selected_instance_identity};
pub(crate) use skeleton::{
    HideMeshesFlag, ShowJointGizmosFlag, SkeletonGizmos, hide_meshes_on_startup,
    setup_skeleton_gizmos_on_top,
};
pub(crate) use solari::{SolariCapability, SolariCapabilityPlugin};
pub(crate) mod extent;
pub(crate) mod visualization;
pub use extent::SceneExtent;
