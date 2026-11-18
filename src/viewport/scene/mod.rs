//! Scene-derived state and semantic presentation inputs.

mod entity_order;
pub(crate) mod hierarchy;
pub(crate) mod index;
pub(in crate::viewport) mod projection_work;

#[allow(unused_imports)]
pub(in crate::viewport) use super::section_box::{
    SectionBoxGizmoTarget, SectionBoxState, SectionClipMaterial, aggregate_selection_bounds,
    capture_section_box_gizmo_transform, draw_section_box, selected_renderable_entities,
    sync_section_box_clipping, sync_section_box_gizmo_target, sync_section_box_state,
};
#[allow(unused_imports)]
pub(in crate::viewport) use super::selection::{
    CoarseSelectionPresentation, CoarseSelectionProxyState, HoverColorMaterial, HoveredTarget,
    ProjectedWorldBounds, SelectedRenderableProjection, SelectionBaseMaterial,
    SelectionColorMaterial, SelectionColorOverride, SelectionColorOverrideState, SelectionOutline,
    SelectionOutlineState, SelectionPresentationPolicy, draw_coarse_selection_proxy,
    register_selection_projection_observers, sync_coarse_selection_proxy,
    sync_selected_renderable_projection, sync_selection_color_overrides, sync_selection_outlines,
};
pub(crate) use super::selection::{SelectedPrim, SelectedTargets, sync_selected_instance_identity};
pub(crate) use crate::viewport::rendering::debug::skeleton::{
    HideMeshesFlag, ShowJointGizmosFlag, SkeletonGizmos, hide_meshes_on_startup,
    setup_skeleton_gizmos_on_top,
};
pub(crate) use crate::viewport::rendering::solari::{SolariCapability, SolariCapabilityPlugin};
pub(in crate::viewport) use entity_order::EntityOrder;
pub(crate) use hierarchy::visibility::refresh_projection_visibility;
pub(crate) use hierarchy::{
    ActiveHierarchyProvider, BimClassificationRecipeState, CurrentHierarchyProjection,
    HierarchyPageIndex, HierarchyVisibilityIndex, HierarchyVisibilityTarget,
};
pub(crate) use index::SceneAnchorIndex;
pub(in crate::viewport) use index::refresh_scene_anchor_index;
pub(crate) use index::register_scene_index_observers;
pub(crate) mod extent;
pub(crate) use crate::viewport::rendering::presentation as visualization;
pub use extent::SceneExtent;
