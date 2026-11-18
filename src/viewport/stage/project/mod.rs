//! Render-host Project activation and LiveStage runtime authority.

use bevy::prelude::App;

pub(in crate::viewport) mod activation;
pub(super) mod live_stage_runtime;
pub(super) mod runtime_authority;
pub(super) mod runtime_snapshot;

pub(in crate::viewport) fn install(app: &mut App) {
    live_stage_runtime::install(app);
    activation::install(app);
}
