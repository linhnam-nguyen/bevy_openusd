//! Main-world admission, activation, and reply flow for Project stages.
//!
//! The preparation worker owns filesystem and cache work. This module owns the
//! main-world ordering rule: admit requests first, then commit only a prepared
//! result that still matches the session's newest command.

use bevy::prelude::World;
use project_protocol::ProjectActivationReply;
use viewport_streaming::ProjectActivationRequest;

use crate::api::RenderServerInterface;
use crate::project::service::ProjectStageActivation;
use crate::viewport::rendering::residency::activation_preview::WarmScenePreview;
use crate::viewport::stage::{
    StageActivationInputLock, StageInstallMode,
    activate_open_stage_with_cache_context_for_generation,
};

use super::support::{
    cache_context_for, publish_activation_result, rollback_cache_bootstrap, scene_owner_id,
};
use super::{
    PreparedProjectActivation, PreparedTarget, ProjectActivationAuthorityRuntime,
    ProjectStageActivationRuntime,
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
        if let Some(pending) = world.remove_resource::<super::PendingCanonicalStageActivation>() {
            abort_deferred(world, &pending);
            let reply = stale_completion_reply(&pending.request.command);
            publish_activation_result(&interface, pending.request, reply);
        }
        let prepare_preview = world.get_non_send::<usd_bevy::LiveStage>().is_some()
            && world.contains_resource::<crate::app::headless::OffscreenTarget>();
        world.insert_resource(input_lock_for(&request));
        let submit_result = world
            .resource::<ProjectStageActivationRuntime>()
            .submit(request, prepare_preview);
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
        release_input_lock(world, &prepared.request);
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
    target: Result<Option<PreparedTarget>, String>,
) -> ActivationProgress {
    let command = request.command.clone();
    if !request_is_current(world, request) {
        return ActivationProgress::Reply(stale_completion_reply(&command));
    }
    if world
        .get_non_send::<usd_bevy::LiveStage>()
        .is_some_and(usd_bevy::LiveStage::is_authoring_frozen)
    {
        return ActivationProgress::Reply(ProjectActivationReply::failed(
            &command,
            "the active document is already leased for publication",
        ));
    }
    if world
        .get_non_send::<usd_bevy::LiveStage>()
        .is_some_and(|stage| !stage.snapshot_persisted_changes().is_empty())
    {
        return ActivationProgress::Reply(ProjectActivationReply::failed(
            &command,
            "Save the active document before switching scenes.",
        ));
    }
    match target {
        Ok(None) => ActivationProgress::Reply(commit_empty_activation(world, request, &command)),
        Ok(Some(target)) => activate_prepared_stage(world, request, &command, target),
        Err(error) => ActivationProgress::Reply(ProjectActivationReply::failed(&command, error)),
    }
}

fn commit_empty_activation(
    world: &mut World,
    request: &ProjectActivationRequest,
    command: &project_protocol::ProjectActivationCommand,
) -> ProjectActivationReply {
    crate::viewport::stage::clear_active_stage_for_generation(world, command.generation);
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
    prepared: PreparedTarget,
) -> ActivationProgress {
    let PreparedTarget {
        target,
        preview,
        _cache_pin,
    } = prepared;
    if let Some(preview) = preview
        && scene_cache_qualified(&target)
        && WarmScenePreview::install(world, command.generation, preview)
    {
        world.insert_resource(super::PendingCanonicalStageActivation {
            request: request.clone(),
            target,
            wait_for_next_update: true,
            warm: true,
            _cache_pin,
        });
        return ActivationProgress::Deferred;
    }
    if let Some(scene_cache) = target.scene_cache.as_ref() {
        let presentation = crate::document::stage_project::stage_presentation_context_from_project(
            target.presentation.clone(),
        );
        let installed =
            crate::viewport::stage::install_scene_cache_bootstrap_with_presentation_before_stage_open(
                world,
                &target.project_root,
                scene_cache,
                Some(presentation),
            );
        if installed {
            world.insert_resource(super::PendingCanonicalStageActivation {
                request: request.clone(),
                target,
                wait_for_next_update: true,
                warm: false,
                _cache_pin,
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
    if !request_is_current(world, request) {
        return ActivationProgress::Reply(stale_completion_reply(command));
    }
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
        activation.into_live_stage(),
        cache_context,
        target.scene_cache.clone(),
        Some(target.project_root.clone()),
        scene_owner_id,
        Some(target.archive_paths.clone()),
        command.generation,
        crate::document::stage_project::stage_presentation_context_from_project(
            target.presentation,
        ),
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

    let can_open = if pending.warm {
        WarmScenePreview::may_open(world, pending.request.command.generation)
    } else if let Some(mut gate) =
        world.get_resource_mut::<crate::viewport::stage::CachePresentationGate>()
    {
        gate.can_open_stage()
    } else if let Some(scene_cache) = pending.target.scene_cache.as_ref() {
        let mut gate = crate::viewport::stage::CachePresentationGate::waiting(
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
    release_input_lock(world, &pending.request);
    if !request_is_current(world, &pending.request) {
        abort_deferred(world, &pending);
        publish_activation_result(
            &interface,
            pending.request,
            stale_completion_reply(&command),
        );
        return;
    }

    let scene_owner_id = scene_owner_id(&pending.target);
    let activation = match ProjectStageActivation::open(&command, pending.target.clone()) {
        Ok(activation) => activation,
        Err(error) => {
            abort_deferred(world, &pending);
            publish_activation_result(
                &interface,
                pending.request,
                ProjectActivationReply::failed(&command, error),
            );
            return;
        }
    };
    let warm_preview = if pending.warm {
        WarmScenePreview::take(world, command.generation)
    } else {
        None
    };
    let mut target = pending.target;
    let stage = activation.into_live_stage();
    if pending.warm && !warm_cache_matches_canonical(&target, &stage) {
        target.scene_cache = None;
    }
    if !request_is_current(world, &pending.request) {
        if !pending.warm {
            rollback_cache_bootstrap(world, &target);
        }
        publish_activation_result(
            &interface,
            pending.request,
            stale_completion_reply(&command),
        );
        return;
    }
    let rollback_target = target.clone();
    let path = target.path.clone();
    let cache_context = cache_context_for(&target);
    let scene_cache = target.scene_cache.clone();
    let project_root = target.project_root.clone();
    let archive_paths = target.archive_paths.clone();
    let presentation = crate::document::stage_project::stage_presentation_context_from_project(
        target.presentation.clone(),
    );
    let result = activate_open_stage_with_cache_context_for_generation(
        world,
        path,
        stage,
        cache_context,
        scene_cache,
        Some(project_root),
        scene_owner_id,
        Some(archive_paths),
        command.generation,
        presentation,
        if pending.warm {
            StageInstallMode::Fresh
        } else {
            StageInstallMode::ContinueCacheFirst
        },
    );
    let reply = match result {
        Ok(())
            if world
                .resource_mut::<ProjectActivationAuthorityRuntime>()
                .0
                .commit(&pending.request.session_id.0, &command) =>
        {
            if scene_cache_qualified(&rollback_target)
                && let Some(preview) = warm_preview
            {
                preview.reuse_meshes(world);
            }
            ProjectActivationReply::activated(&command)
        }
        Ok(()) => {
            if world
                .get_resource::<crate::viewport::stage::StageInfo>()
                .is_some_and(|info| info.activation_generation == command.generation)
            {
                crate::viewport::stage::clear_active_stage_for_generation(
                    world,
                    command.generation,
                );
            }
            stale_completion_reply(&command)
        }
        Err(error) => {
            if !pending.warm {
                rollback_cache_bootstrap(world, &rollback_target);
            }
            ProjectActivationReply::failed(&command, error)
        }
    };
    publish_activation_result(&interface, pending.request, reply);
}

fn scene_cache_qualified(target: &crate::project::service::ProjectStageActivationTarget) -> bool {
    let Some(cache) = &target.scene_cache else {
        return false;
    };
    crate::cache::SceneCacheStore::new(&target.project_root)
        .load_descriptor(cache.descriptor.scene_id)
        .is_ok_and(|current| current.as_ref() == Some(&cache.descriptor))
}

fn warm_cache_matches_canonical(
    target: &crate::project::service::ProjectStageActivationTarget,
    stage: &usd_bevy::LiveStage,
) -> bool {
    let Some(cache) = &target.scene_cache else {
        return false;
    };
    scene_cache_qualified(target)
        && crate::cache::identity::scene_revision_identity_from_snapshot(
            &target.project_root,
            cache.descriptor.scene_id,
            viewport_protocol::RuntimeProfile::NativeMedium,
            cache.descriptor.config_hash,
            stage,
        )
        .is_ok_and(|identity| cache.descriptor.revision_identity == Some(identity.digest()))
}

fn abort_deferred(world: &mut World, pending: &super::PendingCanonicalStageActivation) {
    if pending.warm {
        WarmScenePreview::take(world, pending.request.command.generation);
    } else {
        rollback_cache_bootstrap(world, &pending.target);
    }
}

fn input_lock_for(request: &ProjectActivationRequest) -> StageActivationInputLock {
    StageActivationInputLock {
        session: request.session_id.clone(),
        request_id: request.command.request_id.clone(),
    }
}

fn request_is_current(world: &World, request: &ProjectActivationRequest) -> bool {
    world
        .resource::<ProjectActivationAuthorityRuntime>()
        .0
        .is_current(&request.session_id.0, &request.command)
        && world
            .get_resource::<RenderServerInterface>()
            .is_none_or(|interface| interface.shared().project_activation_is_latest(request))
}

fn release_input_lock(world: &mut World, request: &ProjectActivationRequest) {
    if world
        .get_resource::<StageActivationInputLock>()
        .is_some_and(|lock| {
            lock.session == request.session_id && lock.request_id == request.command.request_id
        })
    {
        world.remove_resource::<StageActivationInputLock>();
    }
}

fn stale_completion_reply(
    command: &project_protocol::ProjectActivationCommand,
) -> ProjectActivationReply {
    ProjectActivationReply::failed(command, "stale Project activation completion was ignored")
}

#[cfg(test)]
#[path = "../../../../tests/unit/viewport/stage/project/activation/flow_test_support.rs"]
pub(super) mod test_support;
