//! Main-world admission, activation, and reply flow for Project stages.
//!
//! The preparation worker owns filesystem and cache work. This module owns the
//! main-world ordering rule: admit requests first, then commit only a prepared
//! result that still matches the session's newest command.

use bevy::prelude::World;
use project_protocol::ProjectActivationReply;
use viewport_streaming::{
    ProjectActivationRequest, ProjectActivationResult as RoutedProjectActivationResult,
};

use crate::project::cache::ProjectCacheTarget;
use crate::project::cache_hydration::{
    ActiveProjectCacheContext, default_project_cache_config_hash,
};
use crate::viewport::api::RenderServerInterface;
use crate::viewport::session::StagePresentationContext;
use crate::viewport::session::activate_stage_with_cache_context_for_generation;

use super::{
    PreparedProjectActivation, ProjectActivationAuthorityRuntime, ProjectStageActivationRuntime,
};

/// Submits queued Project activations for preparation and applies prepared
/// results on the Bevy main world.
///
/// The candidate Stage is opened before the current LiveStage is replaced, so
/// a failed activation leaves the previous renderer state untouched.
pub(super) fn process_project_activations(world: &mut World) {
    let Some(interface_resource) = world.get_resource::<RenderServerInterface>() else {
        return;
    };
    let interface = interface_resource.shared();

    while let Some(request) = interface.pop_project_activation() {
        let admitted = world
            .resource_mut::<ProjectActivationAuthorityRuntime>()
            .0
            .observe_request(&request.session_id.0, &request.command);
        if !admitted {
            let command = request.command.clone();
            publish_activation_result(
                &interface,
                request,
                ProjectActivationReply::failed(
                    &command,
                    "stale Project activation request was ignored",
                ),
            );
            continue;
        }
        let submit_result = world
            .resource::<ProjectStageActivationRuntime>()
            .submit(request);
        if let Err(error) = submit_result {
            let (request, message) = match error {
                std::sync::mpsc::TrySendError::Full(request) => {
                    (request, "Project activation preparation is busy".to_owned())
                }
                std::sync::mpsc::TrySendError::Disconnected(request) => (
                    request,
                    "Project activation preparation is unavailable".to_owned(),
                ),
            };
            let command = request.command.clone();
            publish_activation_result(
                &interface,
                request,
                ProjectActivationReply::failed(&command, message),
            );
        }
    }

    while let Some(prepared) = world
        .resource::<ProjectStageActivationRuntime>()
        .take_prepared()
    {
        publish_prepared_result(world, &interface, prepared);
    }
}

fn publish_prepared_result(
    world: &mut World,
    interface: &viewport_streaming::RenderServerInterface,
    prepared: PreparedProjectActivation,
) {
    let command = prepared.request.command.clone();
    if !world
        .resource::<ProjectActivationAuthorityRuntime>()
        .0
        .is_current(&prepared.request.session_id.0, &command)
    {
        publish_activation_result(
            interface,
            prepared.request,
            ProjectActivationReply::failed(
                &command,
                "stale Project activation completion was ignored",
            ),
        );
        return;
    }
    let reply = match prepared.target {
        Ok(None) => commit_empty_activation(world, &prepared.request, &command),
        Ok(Some(target)) => activate_prepared_stage(world, &prepared.request, &command, target),
        Err(error) => ProjectActivationReply::failed(&command, error),
    };
    publish_activation_result(interface, prepared.request, reply);
}

fn commit_empty_activation(
    world: &mut World,
    request: &ProjectActivationRequest,
    command: &project_protocol::ProjectActivationCommand,
) -> ProjectActivationReply {
    if world
        .resource_mut::<ProjectActivationAuthorityRuntime>()
        .0
        .commit(&request.session_id.0, command)
    {
        ProjectActivationReply::activated(command)
    } else {
        stale_completion_reply(command)
    }
}

fn activate_prepared_stage(
    world: &mut World,
    request: &ProjectActivationRequest,
    command: &project_protocol::ProjectActivationCommand,
    target: crate::project::service::ProjectStageActivationTarget,
) -> ProjectActivationReply {
    let cache_context = cache_context_for(&target);
    match activate_stage_with_cache_context_for_generation(
        world,
        target.path,
        cache_context,
        command.generation,
        StagePresentationContext::from_project(target.presentation),
    ) {
        Ok(()) => {
            if world
                .resource_mut::<ProjectActivationAuthorityRuntime>()
                .0
                .commit(&request.session_id.0, command)
            {
                ProjectActivationReply::activated(command)
            } else {
                stale_completion_reply(command)
            }
        }
        Err(error) => ProjectActivationReply::failed(command, error),
    }
}

fn stale_completion_reply(
    command: &project_protocol::ProjectActivationCommand,
) -> ProjectActivationReply {
    ProjectActivationReply::failed(command, "stale Project activation completion was ignored")
}

fn cache_context_for(
    target: &crate::project::service::ProjectStageActivationTarget,
) -> Option<ActiveProjectCacheContext> {
    let cache_target = match target.target {
        project_protocol::ProjectStageTarget::ProjectRoot(_) => ProjectCacheTarget::ProjectRoot,
        project_protocol::ProjectStageTarget::Scene(scene_id) => ProjectCacheTarget::Scene {
            id: scene_id.to_string(),
        },
        project_protocol::ProjectStageTarget::Model(model_id) => ProjectCacheTarget::Model {
            id: model_id.to_string(),
        },
    };
    match ActiveProjectCacheContext::new(
        target.project_root.clone(),
        cache_target,
        viewport_protocol::RuntimeProfile::NativeMedium,
        default_project_cache_config_hash(),
    ) {
        Ok(context) => Some(context),
        Err(error) => {
            bevy::log::warn!(
                "[project-cache] could not establish activation identity; using source projection: {error:#}"
            );
            None
        }
    }
}

fn publish_activation_result(
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
