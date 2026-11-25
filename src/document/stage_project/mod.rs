//! Render-host Project activation and LiveStage runtime authority.

use bevy::prelude::App;

pub(crate) mod activation;
pub(super) mod live_stage_runtime;
pub(super) mod runtime_authority;
pub(super) mod runtime_snapshot;

pub(crate) fn install(app: &mut App) {
    app.insert_resource(crate::api::product_stage_lifecycle_hooks());
    live_stage_runtime::install(app);
    activation::install(app);
}

pub(crate) fn stage_presentation_context_from_project(
    context: crate::project::service::ProjectStagePresentationContext,
) -> crate::viewport::stage::StagePresentationContext {
    crate::viewport::stage::StagePresentationContext {
        root_path: context.root_path,
        root_name: context.root_name,
        target_names: context.target_names,
    }
}
