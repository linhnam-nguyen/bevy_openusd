use bevy::prelude::World;
use project_protocol::ProjectActivationReply;
use viewport_streaming::{ProjectActivationRequest, ProjectActivationResult as RoutedProjectActivationResult};

use crate::project::cache_hydration::ActiveProjectCacheContext;

pub(super) fn scene_owner_id(
    target: &crate::project::service::ProjectStageActivationTarget,
) -> Option<usd_project::SceneId> {
    match &target.target {
        project_protocol::ProjectStageTarget::Scene(scene_id)
        | project_protocol::ProjectStageTarget::ProjectRoot(usd_project::ProjectRoot::Scene(
            scene_id,
        )) => Some(*scene_id),
        _ => None,
    }
}

pub(super) fn rollback_cache_bootstrap(
    world: &mut World,
    target: &crate::project::service::ProjectStageActivationTarget,
) {
    world.remove_resource::<crate::viewport::session::CachePresentationGate>();
    if let Some(scene_cache) = target.scene_cache.as_ref() {
        crate::viewport::session::discard_scene_cache_bootstrap(
            world,
            scene_cache.descriptor.scene_id,
            scene_cache.descriptor.generation,
        );
    }
}

pub(super) fn cache_context_for(
    target: &crate::project::service::ProjectStageActivationTarget,
) -> Option<ActiveProjectCacheContext> {
    target.cache_identity.clone().map(|identity| {
        ActiveProjectCacheContext::from_identity(target.project_root.clone(), identity)
    })
}

pub(super) fn publish_activation_result(
    interface: &viewport_streaming::RenderServerInterface,
    request: ProjectActivationRequest,
    reply: ProjectActivationReply,
) {
    let result = RoutedProjectActivationResult {
        session_id: request.session_id,
        reply,
    };
    if let Err(error) = interface.publish_project_activation_result(result) {
        bevy::log::error!("[project-activation] could not publish activation result: {error:?}");
    }
}
