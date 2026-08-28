//! Current model source, load lifecycle, and stage-derived read model.

mod lifecycle;
mod state;

pub(crate) use lifecycle::{
    activate_stage, apply_load_request, handle_usd_hot_reload, load_stage, spawn_when_ready,
};
pub(crate) use state::{
    LoadRequest, LoaderTuning, ReloadRequest, RequestedAsset, Spawned, StageCameraData,
    StageCameraInfo, StageCameraProjection, StageHandle, StageInfo, VariantSetInfo,
};
