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

use crate::project::cache_hydration::ActiveProjectCacheContext;
use crate::project::service::ProjectStageActivation;
use crate::viewport::api::RenderServerInterface;
use crate::viewport::session::{
    StagePresentationContext, activate_open_stage_with_cache_context_for_generation,
};

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
        if let Some(mut projection) =
            world.get_resource_mut::<usd_bevy::ProgressiveProjectionState>()
        {
            projection.cancel();
        }
        let submit_result = world
            .resource::<ProjectStageActivationRuntime>()
            .submit(request);
        if let Some(request) = submit_result {
            let command = request.command.clone();
            publish_activation_result(
                &interface,
                request,
                ProjectActivationReply::failed(
                    &command,
                    "stale Project activation was superseded by a newer request",
                ),
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
    let reply = apply_prepared_activation(world, &prepared.request, prepared.target);
    publish_activation_result(interface, prepared.request, reply);
}

/// Applies one prepared completion through the production Bevy-world
/// authority. The currency check is intentionally before Stage installation,
/// so a late completion cannot mutate any stage-derived resource.
fn apply_prepared_activation(
    world: &mut World,
    request: &ProjectActivationRequest,
    target: Result<Option<crate::project::service::ProjectStageActivationTarget>, String>,
) -> ProjectActivationReply {
    let command = request.command.clone();
    if !world
        .resource::<ProjectActivationAuthorityRuntime>()
        .0
        .is_current(&request.session_id.0, &command)
    {
        return stale_completion_reply(&command);
    }
    match target {
        Ok(None) => commit_empty_activation(world, request, &command),
        Ok(Some(target)) => activate_prepared_stage(world, request, &command, target),
        Err(error) => ProjectActivationReply::failed(&command, error),
    }
}

#[cfg(test)]
pub(crate) fn apply_prepared_activation_for_test(
    world: &mut World,
    request: &ProjectActivationRequest,
    target: Result<Option<crate::project::service::ProjectStageActivationTarget>, String>,
) -> ProjectActivationReply {
    apply_prepared_activation(world, request, target)
}

fn commit_empty_activation(
    world: &mut World,
    request: &ProjectActivationRequest,
    command: &project_protocol::ProjectActivationCommand,
) -> ProjectActivationReply {
    crate::viewport::session::clear_active_stage_for_generation(world, command.generation);
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
    let activation = match ProjectStageActivation::open(command, target.clone()) {
        Ok(activation) => activation,
        Err(error) => return ProjectActivationReply::failed(command, error),
    };
    let cache_context = cache_context_for(&target);
    match activate_open_stage_with_cache_context_for_generation(
        world,
        target.path,
        activation.into_stage(),
        cache_context,
        Some(target.archive_paths.clone()),
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
    target.cache_identity.clone().map(|identity| {
        ActiveProjectCacheContext::from_identity(target.project_root.clone(), identity)
    })
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
