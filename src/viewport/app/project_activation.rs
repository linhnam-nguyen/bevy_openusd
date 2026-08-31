//! Render-host Project activation boundary.
//!
//! Transport code submits stable Project identity. This module is the only
//! place where that identity is resolved through the Project application
//! service and handed to the existing LiveStage stage-open lifecycle.

use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex,
        mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
};

use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::prelude::{App, Resource, Update, World};
use project_protocol::ProjectActivationReply;
use viewport_streaming::{
    ProjectActivationRequest, ProjectActivationResult as RoutedProjectActivationResult,
};

use crate::project::cache::ProjectCacheTarget;
use crate::project::cache_hydration::{
    ActiveProjectCacheContext, default_project_cache_config_hash,
};
use crate::project::service::ProjectApplicationService;
use crate::viewport::api::RenderServerInterface;
use crate::viewport::session::StagePresentationContext;
use crate::viewport::session::activate_stage_with_cache_context_for_generation;

const PROJECT_REGISTRY_PATH_ENV: &str = "USDHUB_PROJECT_WORKSPACE_REGISTRY";
const PROJECT_ACTIVATION_PREPARATION_CAPACITY: usize = 2;

/// Host-owned locator for the machine-local Project registry.
#[derive(Resource)]
pub(super) struct ProjectStageActivationRuntime {
    preparation: ProjectActivationPreparation,
}

struct ProjectActivationPreparation {
    sender: SyncSender<ProjectActivationRequest>,
    receiver: Mutex<Receiver<PreparedProjectActivation>>,
}

struct PreparedProjectActivation {
    request: ProjectActivationRequest,
    target: Result<Option<crate::project::service::ProjectStageActivationTarget>, String>,
}

impl ProjectStageActivationRuntime {
    pub(super) fn from_environment() -> Self {
        let registry_path = std::env::var_os(PROJECT_REGISTRY_PATH_ENV).map(PathBuf::from);
        if registry_path.is_none() {
            bevy::log::warn!(
                "[project-activation] {PROJECT_REGISTRY_PATH_ENV} is not configured; Project stage activation is unavailable"
            );
        }
        Self::with_registry_path(registry_path)
    }

    fn with_registry_path(registry_path: Option<PathBuf>) -> Self {
        let (sender, requests) = sync_channel(PROJECT_ACTIVATION_PREPARATION_CAPACITY);
        let (prepared, receiver) = sync_channel(PROJECT_ACTIVATION_PREPARATION_CAPACITY);
        std::thread::Builder::new()
            .name("project-activation-preparation".to_owned())
            .spawn(move || {
                while let Ok(request) = requests.recv() {
                    let target = resolve_project_activation(registry_path.as_deref(), &request);
                    if prepared
                        .send(PreparedProjectActivation { request, target })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("Project activation preparation worker must start");

        Self {
            preparation: ProjectActivationPreparation {
                sender,
                receiver: Mutex::new(receiver),
            },
        }
    }

    fn submit(
        &self,
        request: ProjectActivationRequest,
    ) -> Result<(), TrySendError<ProjectActivationRequest>> {
        self.preparation.sender.try_send(request)
    }

    fn take_prepared(&self) -> Option<PreparedProjectActivation> {
        let receiver = self
            .preparation
            .receiver
            .lock()
            .expect("Project activation preparation receiver is not poisoned");
        match receiver.try_recv() {
            Ok(prepared) => Some(prepared),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }

    #[cfg(test)]
    fn wait_for_prepared(&self) -> Option<PreparedProjectActivation> {
        self.preparation
            .receiver
            .lock()
            .expect("Project activation preparation receiver is not poisoned")
            .recv_timeout(std::time::Duration::from_secs(1))
            .ok()
    }
}

fn resolve_project_activation(
    registry_path: Option<&Path>,
    request: &ProjectActivationRequest,
) -> Result<Option<crate::project::service::ProjectStageActivationTarget>, String> {
    let Some(registry_path) = registry_path else {
        return Err("Project activation registry is unavailable".to_owned());
    };
    let service = ProjectApplicationService::open(registry_path.to_path_buf())
        .map_err(|error| format!("Project activation service is unavailable: {error}"))?;
    let target = service
        .resolve_stage_activation(request.command.project_id, request.command.target.clone())
        .map_err(|error| format!("Project stage activation was rejected: {error}"))?;
    if let Some(target) = target.as_ref() {
        let cache_preparation = service.prepare_cache_for_activation(target);
        match cache_preparation {
            crate::project::cache_warmer::ProjectCachePreparation::Ready => {}
            crate::project::cache_warmer::ProjectCachePreparation::Empty => {
                bevy::log::debug!("[project-cache] activation target is empty")
            }
            crate::project::cache_warmer::ProjectCachePreparation::FallbackRequired => {
                bevy::log::debug!(
                    "[project-cache] activation cache unavailable; canonical source fallback remains active"
                )
            }
            crate::project::cache_warmer::ProjectCachePreparation::TimedOut => {
                bevy::log::warn!(
                    "[project-cache] activation cache preparation timed out; canonical source fallback remains active"
                )
            }
        }
    }
    Ok(target)
}

pub(super) fn install(app: &mut App) {
    app.insert_resource(ProjectStageActivationRuntime::from_environment())
        .add_systems(
            Update,
            process_project_activations.before(crate::viewport::session::spawn_when_ready),
        );
}

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

    loop {
        let prepared = world
            .resource::<ProjectStageActivationRuntime>()
            .take_prepared();
        let Some(prepared) = prepared else {
            break;
        };
        publish_prepared_result(world, &interface, prepared);
    }

    while let Some(request) = interface.pop_project_activation() {
        let submit_result = world
            .resource::<ProjectStageActivationRuntime>()
            .submit(request);
        if let Err(error) = submit_result {
            let (request, message) = match error {
                TrySendError::Full(request) => {
                    (request, "Project activation preparation is busy".to_owned())
                }
                TrySendError::Disconnected(request) => (
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
}

fn publish_prepared_result(
    world: &mut World,
    interface: &viewport_streaming::RenderServerInterface,
    prepared: PreparedProjectActivation,
) {
    let command = prepared.request.command.clone();
    let reply = match prepared.target {
        Ok(None) => ProjectActivationReply::activated(&command),
        Ok(Some(target)) => {
            let cache_context = cache_context_for(&target);
            match activate_stage_with_cache_context_for_generation(
                world,
                target.path,
                cache_context,
                command.generation,
                StagePresentationContext::from_project(target.presentation),
            ) {
                Ok(()) => ProjectActivationReply::activated(&command),
                Err(error) => ProjectActivationReply::failed(&command, error),
            }
        }
        Err(error) => ProjectActivationReply::failed(&command, error),
    };
    publish_activation_result(interface, prepared.request, reply);
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

#[cfg(test)]
mod tests {
    use super::*;
    use project_protocol::{ProjectActivationCommand, ProjectActivationReply, ProjectStageTarget};
    use usd_project::{ProjectId, ProjectRoot, SceneId};
    use viewport_protocol::SessionId;

    #[test]
    fn activation_reply_preserves_project_root_generation_and_has_no_path() {
        let project_id = ProjectId::new_v4();
        let command = ProjectActivationCommand::new(
            "activation-b",
            7,
            project_id,
            ProjectStageTarget::ProjectRoot(ProjectRoot::Scene(SceneId::new_v4())),
        );
        let reply = ProjectActivationReply::activated(&command);
        let encoded = serde_json::to_string(&reply).unwrap();

        assert!(encoded.contains("activation-b"));
        assert!(encoded.contains(&project_id.to_string()));
        assert!(!encoded.contains("/Users/"));
    }

    #[test]
    fn activation_requests_are_routed_to_the_requesting_session() {
        let interface = RenderServerInterface::default();
        let project_id = ProjectId::new_v4();
        let command = ProjectActivationCommand::new(
            "activation-a",
            1,
            project_id,
            ProjectStageTarget::ProjectRoot(ProjectRoot::Empty),
        );
        interface
            .submit_project_activation(ProjectActivationRequest {
                session_id: SessionId::new("session-a"),
                command: command.clone(),
            })
            .unwrap();
        let request = interface.pop_project_activation().unwrap();
        interface
            .publish_project_activation_result(RoutedProjectActivationResult {
                session_id: request.session_id.clone(),
                reply: ProjectActivationReply::activated(&request.command),
            })
            .unwrap();

        let result = interface
            .take_project_activation_result(&SessionId::new("session-a"))
            .unwrap();
        assert_eq!(result.session_id, SessionId::new("session-a"));
        assert_eq!(result.reply.request_id, command.request_id);
    }

    #[test]
    fn preparation_worker_preserves_request_identity_when_registry_is_unavailable() {
        let runtime = ProjectStageActivationRuntime::with_registry_path(None);
        let command = ProjectActivationCommand::new(
            "activation-preparation-failure",
            4,
            ProjectId::new_v4(),
            ProjectStageTarget::ProjectRoot(ProjectRoot::Empty),
        );
        let request = ProjectActivationRequest {
            session_id: SessionId::new("session-preparation"),
            command: command.clone(),
        };

        runtime.submit(request.clone()).unwrap();
        let prepared = runtime
            .wait_for_prepared()
            .expect("preparation worker should return a result");
        assert_eq!(prepared.request.session_id, request.session_id);
        assert_eq!(prepared.request.command, command);
        assert_eq!(
            prepared.target,
            Err("Project activation registry is unavailable".to_owned())
        );
    }
}

#[cfg(test)]
#[path = "project_activation_cache_tests.rs"]
mod cache_tests;
