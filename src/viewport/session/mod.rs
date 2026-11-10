//! Current model source, load lifecycle, and stage-derived read model.

#[path = "cache_presentation_gate.rs"]
mod cache_presentation_gate;
mod lifecycle;
mod state;

pub(crate) use cache_presentation_gate::CachePresentationGate;
pub(in crate::viewport) use lifecycle::{
    PendingActivationPresentation, rehydrate_activation_presentation,
};
pub(crate) use lifecycle::{
    StageInstallMode, activate_open_stage_with_cache_context_for_generation, activate_stage,
    activate_stage_with_cache_context, activate_stage_with_cache_context_for_generation,
    apply_load_request, clear_active_stage_for_generation, discard_scene_cache_bootstrap,
    handle_usd_hot_reload, install_scene_cache_bootstrap_before_stage_open, load_stage,
    poll_scene_cache_revalidation, spawn_when_ready,
};
pub(crate) use state::{
    LoadRequest, LoaderTuning, PendingSceneCacheRevalidation, ReloadRequest, RequestedAsset,
    SceneCacheOwnershipContext, SceneCachePresentation, SceneDerivedMetadata, Spawned,
    StageCameraData, StageCameraInfo, StageCameraProjection, StageHandle, StageInfo,
    StagePresentationContext, VariantSetInfo,
};
