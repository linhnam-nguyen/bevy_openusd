//! Main-world admission, activation, and reply flow for Project stages.
//!
//! The preparation worker owns filesystem and cache work. This module owns the
//! main-world ordering rule: admit requests first, then commit only a prepared
//! result that still matches the session's newest command.

use bevy::prelude::World;
use project_protocol::ProjectActivationReply;
use viewport_streaming::ProjectActivationRequest;

use crate::project::service::ProjectStageActivation;
use crate::viewport::api::RenderServerInterface;
use crate::viewport::session::{
    StageInstallMode, StagePresentationContext,
    activate_open_stage_with_cache_context_for_generation,
};

use super::{
    PreparedProjectActivation, ProjectActivationAuthorityRuntime, ProjectStageActivationRuntime,
};
use helpers::{
    cache_context_for, publish_activation_result, rollback_cache_bootstrap, scene_owner_id,
};

#[path = "project_activation_flow_helpers.rs"]
mod helpers;

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
        if let Some(pending) = world.remove_resource::<super::PendingCanonicalStageActivation>() {
            rollback_cache_bootstrap(world, &pending.target);
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
    if let ActivationProgress::Reply(reply) =
        apply_prepared_activation(world, &prepared.request, prepared.target)
    {
        publish_activation_result(interface, prepared.request, reply);
    }
}

enum ActivationProgress {
    Reply(ProjectActivationReply),
    Deferred,
}

/// Applies one prepared completion through the production Bevy-world
/// authority. The currency check is intentionally before Stage installation,
/// so a late completion cannot mutate any stage-derived resource.
fn apply_prepared_activation(
    world: &mut World,
    request: &ProjectActivationRequest,
    target: Result<Option<crate::project::service::ProjectStageActivationTarget>, String>,
) -> ActivationProgress {
    let command = request.command.clone();
    if !world
        .resource::<ProjectActivationAuthorityRuntime>()
        .0
        .is_current(&request.session_id.0, &command)
    {
        return ActivationProgress::Reply(stale_completion_reply(&command));
    }
    match target {
        Ok(None) => ActivationProgress::Reply(commit_empty_activation(world, request, &command)),
        Ok(Some(target)) => activate_prepared_stage(world, request, &command, target),
        Err(error) => ActivationProgress::Reply(ProjectActivationReply::failed(&command, error)),
    }
}

#[cfg(test)]
pub(crate) fn apply_prepared_activation_for_test(
    world: &mut World,
    request: &ProjectActivationRequest,
    target: Result<Option<crate::project::service::ProjectStageActivationTarget>, String>,
) -> Option<ProjectActivationReply> {
    match apply_prepared_activation(world, request, target) {
        ActivationProgress::Reply(reply) => Some(reply),
        ActivationProgress::Deferred => None,
    }
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
) -> ActivationProgress {
    if let Some(scene_cache) = target.scene_cache.as_ref() {
        let installed = crate::viewport::session::install_scene_cache_bootstrap_before_stage_open(
            world,
            &target.project_root,
            scene_cache,
        );
        if installed {
            world.insert_resource(super::PendingCanonicalStageActivation {
                request: request.clone(),
                target,
                wait_for_next_update: true,
            });
            return ActivationProgress::Deferred;
        }
    }
    let activation = match ProjectStageActivation::open(command, target.clone()) {
        Ok(activation) => activation,
        Err(error) => {
            return ActivationProgress::Reply(ProjectActivationReply::failed(command, error));
        }
    };
    let cache_context = cache_context_for(&target);
    let scene_owner_id = match &target.target {
        project_protocol::ProjectStageTarget::Scene(scene_id)
        | project_protocol::ProjectStageTarget::ProjectRoot(usd_project::ProjectRoot::Scene(
            scene_id,
        )) => Some(scene_id.clone()),
        _ => None,
    };
    match activate_open_stage_with_cache_context_for_generation(
        world,
        target.path,
        activation.into_stage(),
        cache_context,
        target.scene_cache.clone(),
        Some(target.project_root.clone()),
        scene_owner_id,
        Some(target.archive_paths.clone()),
        command.generation,
        StagePresentationContext::from_project(target.presentation),
        StageInstallMode::Fresh,
    ) {
        Ok(()) => {
            if world
                .resource_mut::<ProjectActivationAuthorityRuntime>()
                .0
                .commit(&request.session_id.0, command)
            {
                ActivationProgress::Reply(ProjectActivationReply::activated(command))
            } else {
                ActivationProgress::Reply(stale_completion_reply(command))
            }
        }
        Err(error) => ActivationProgress::Reply(ProjectActivationReply::failed(command, error)),
    }
}

/// Opens a cache-first canonical Stage only after the bootstrap update has
/// completed. The final protocol reply is emitted here, never at bootstrap.
pub(super) fn continue_deferred_stage_activation(world: &mut World) {
    let Some(mut pending) = world.remove_resource::<super::PendingCanonicalStageActivation>()
    else {
        return;
    };
    if pending.wait_for_next_update {
        pending.wait_for_next_update = false;
        world.insert_resource(pending);
        return;
    }

    let can_open = if let Some(mut gate) =
        world.get_resource_mut::<crate::viewport::session::CachePresentationGate>()
    {
        gate.can_open_stage()
    } else if let Some(scene_cache) = pending.target.scene_cache.as_ref() {
        let mut gate = crate::viewport::session::CachePresentationGate::waiting(
            scene_cache.descriptor.scene_id,
            scene_cache.descriptor.generation,
        );
        let can_open = gate.can_open_stage();
        world.insert_resource(gate);
        can_open
    } else {
        true
    };
    if !can_open {
        world.insert_resource(pending);
        return;
    }

    let Some(interface) = world
        .get_resource::<RenderServerInterface>()
        .map(RenderServerInterface::shared)
    else {
        world.insert_resource(pending);
        return;
    };
    let command = pending.request.command.clone();
    if !world
        .resource::<ProjectActivationAuthorityRuntime>()
        .0
        .is_current(&pending.request.session_id.0, &command)
    {
        rollback_cache_bootstrap(world, &pending.target);
        publish_activation_result(
            &interface,
            pending.request,
            stale_completion_reply(&command),
        );
        return;
    }

    let scene_owner_id = scene_owner_id(&pending.target);
    let target = pending.target;
    let rollback_target = target.clone();
    let activation = match ProjectStageActivation::open(&command, target.clone()) {
        Ok(activation) => activation,
        Err(error) => {
            rollback_cache_bootstrap(world, &target);
            publish_activation_result(
                &interface,
                pending.request,
                ProjectActivationReply::failed(&command, error),
            );
            return;
        }
    };
    if !world
        .resource::<ProjectActivationAuthorityRuntime>()
        .0
        .is_current(&pending.request.session_id.0, &command)
    {
        rollback_cache_bootstrap(world, &target);
        publish_activation_result(
            &interface,
            pending.request,
            stale_completion_reply(&command),
        );
        return;
    }

    let path = target.path.clone();
    let cache_context = cache_context_for(&target);
    let scene_cache = target.scene_cache.clone();
    let project_root = target.project_root.clone();
    let archive_paths = target.archive_paths.clone();
    let presentation = StagePresentationContext::from_project(target.presentation.clone());
    let result = activate_open_stage_with_cache_context_for_generation(
        world,
        path,
        activation.into_stage(),
        cache_context,
        scene_cache,
        Some(project_root),
        scene_owner_id,
        Some(archive_paths),
        command.generation,
        presentation,
        StageInstallMode::ContinueCacheFirst,
    );
    let reply = match result {
        Ok(()) if world
            .resource_mut::<ProjectActivationAuthorityRuntime>()
            .0
            .commit(&pending.request.session_id.0, &command) =>
        ProjectActivationReply::activated(&command),
        Ok(()) => {
            if world
                .get_resource::<crate::viewport::session::StageInfo>()
                .is_some_and(|info| info.activation_generation == command.generation)
            {
                crate::viewport::session::clear_active_stage_for_generation(
                    world,
                    command.generation,
                );
            }
            stale_completion_reply(&command)
        }
        Err(error) => {
            rollback_cache_bootstrap(world, &rollback_target);
            ProjectActivationReply::failed(&command, error)
        }
    };
    publish_activation_result(&interface, pending.request, reply);
}

#[cfg(test)]
pub(crate) fn continue_deferred_stage_activation_for_test(world: &mut World) {
    continue_deferred_stage_activation(world);
}

fn stale_completion_reply(
    command: &project_protocol::ProjectActivationCommand,
) -> ProjectActivationReply {
    ProjectActivationReply::failed(command, "stale Project activation completion was ignored")
}
