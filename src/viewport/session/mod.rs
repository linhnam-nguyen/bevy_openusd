//! Current model source, load lifecycle, and stage-derived read model.

mod lifecycle;
mod state;

pub(in crate::viewport) use lifecycle::{
    PendingActivationPresentation, rehydrate_activation_presentation,
};
pub(crate) use lifecycle::{
    activate_open_stage_with_cache_context_for_generation, activate_stage,
    activate_stage_with_cache_context, activate_stage_with_cache_context_for_generation,
    apply_load_request, clear_active_stage_for_generation, handle_usd_hot_reload, load_stage,
    poll_scene_cache_revalidation, publish_scene_cache_presentation_before_stage_open,
    spawn_when_ready,
};
pub(crate) use state::{
    LoadRequest, LoaderTuning, PendingSceneCacheRevalidation, ReloadRequest, RequestedAsset,
    SceneCacheOwnershipContext, SceneCachePresentation, SceneDerivedMetadata, Spawned,
    StageCameraData, StageCameraInfo, StageCameraProjection, StageHandle, StageInfo,
    StagePresentationContext, VariantSetInfo,
};
