//! Current model source, load lifecycle, and stage-derived read model.

mod lifecycle;
mod state;
#[path = "cache_presentation_gate.rs"]
mod cache_presentation_gate;
#[path = "visual_handoff_state.rs"]
mod visual_handoff_state;

pub(in crate::viewport) use lifecycle::{
    PendingActivationPresentation, rehydrate_activation_presentation,
};
pub(crate) use lifecycle::{
    activate_open_stage_with_cache_context_for_generation, activate_stage,
    activate_stage_with_cache_context, activate_stage_with_cache_context_for_generation,
    apply_load_request, clear_active_stage_for_generation, handle_usd_hot_reload, load_stage,
    discard_scene_cache_bootstrap, install_scene_cache_bootstrap_before_stage_open,
    poll_scene_cache_revalidation,
    spawn_when_ready, StageInstallMode,
};
pub(crate) use state::{
    LoadRequest, LoaderTuning, PendingSceneCacheRevalidation, ReloadRequest, RequestedAsset,
    SceneCacheOwnershipContext, SceneCachePresentation, SceneDerivedMetadata, Spawned,
    StageCameraData, StageCameraInfo, StageCameraProjection, StageHandle, StageInfo,
    StagePresentationContext, VariantSetInfo,
};
pub(crate) use visual_handoff_state::PendingCanonicalVisualHandoff;
pub(crate) use cache_presentation_gate::CachePresentationGate;
