//! Render-host Project activation boundary.
//!
//! Transport code submits stable Project identity. This module is the only
//! place where that identity is resolved through the Project application
//! service and handed to the existing LiveStage stage-open lifecycle.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, TryRecvError, sync_channel},
    },
};

use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::prelude::{App, Resource, Update};
use viewport_streaming::ProjectActivationRequest;

use crate::project::cache_warmer::ProjectCacheWarmQueue;
use crate::project::service::ProjectActivationAuthority;
use crate::project::service::ProjectApplicationService;

#[path = "project_activation_flow.rs"]
mod flow;
#[path = "project_activation_queue.rs"]
mod queue;

use queue::{ActivationCancellation, LatestActivationQueue};

const PROJECT_REGISTRY_PATH_ENV: &str = "USDHUB_PROJECT_WORKSPACE_REGISTRY";
/// Host-owned locator for the machine-local Project registry.
#[derive(Resource)]
pub(super) struct ProjectStageActivationRuntime {
    preparation: ProjectActivationPreparation,
    _cache_warm: ProjectCacheWarmQueue,
}

#[derive(Resource, Default)]
pub(super) struct ProjectActivationAuthorityRuntime(ProjectActivationAuthority);

struct ProjectActivationPreparation {
    requests: Arc<LatestActivationQueue>,
    receiver: Mutex<Receiver<PreparedProjectActivation>>,
    cancellation: Arc<ActivationCancellation>,
}

struct PreparedProjectActivation {
    request: ProjectActivationRequest,
    target: Result<Option<crate::project::service::ProjectStageActivationTarget>, String>,
}

impl Drop for ProjectStageActivationRuntime {
    fn drop(&mut self) {
        self.preparation.requests.close();
        self._cache_warm.shutdown_without_waiting();
    }
}

impl Drop for ProjectActivationPreparation {
    fn drop(&mut self) {
        self.requests.close();
    }
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
        let requests = Arc::new(LatestActivationQueue::new());
        let (prepared, receiver) = sync_channel(2);
        let cancellation = Arc::new(ActivationCancellation::default());
        let cache_warm = ProjectCacheWarmQueue::default();
        let worker_requests = Arc::clone(&requests);
        let worker_cancellation = Arc::clone(&cancellation);
        let worker_cache_warm = cache_warm.clone();
        std::thread::Builder::new()
            .name("project-activation-preparation".to_owned())
            .spawn(move || {
                while let Some(queued) = worker_requests.take() {
                    let queue_wait_ms = queued.enqueued_at.elapsed().as_secs_f64() * 1_000.0;
                    let request = queued.request;
                    bevy::log::debug!(
                        "[project-loading] activation_queue_wait_ms={queue_wait_ms:.3} request={}",
                        request.command.request_id
                    );
                    let target = resolve_project_activation_with_cache(
                        registry_path.as_deref(),
                        &request,
                        &worker_cache_warm,
                        &worker_cancellation,
                    );
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
                requests,
                receiver: Mutex::new(receiver),
                cancellation,
            },
            _cache_warm: cache_warm,
        }
    }

    fn submit(&self, request: ProjectActivationRequest) -> Option<ProjectActivationRequest> {
        self.preparation.cancellation.supersede(&request);
        self.preparation.requests.replace(request)
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
    let cache_warm = ProjectCacheWarmQueue::default();
    let cancellation = ActivationCancellation::default();
    cancellation.supersede(request);
    resolve_project_activation_with_cache(registry_path, request, &cache_warm, &cancellation)
}

fn resolve_project_activation_with_cache(
    registry_path: Option<&Path>,
    request: &ProjectActivationRequest,
    cache_warm: &ProjectCacheWarmQueue,
    cancellation: &ActivationCancellation,
) -> Result<Option<crate::project::service::ProjectStageActivationTarget>, String> {
    let Some(registry_path) = registry_path else {
        return Err("Project activation registry is unavailable".to_owned());
    };
    if !cancellation.is_current(request) {
        return Err("stale Project activation preparation was superseded".to_owned());
    }
    let service = ProjectApplicationService::open_with_cache_warm(
        registry_path.to_path_buf(),
        cache_warm.clone(),
    )
    .map_err(|error| format!("Project activation service is unavailable: {error}"))?;
    let target = service
        .resolve_stage_activation(request.command.project_id, request.command.target.clone())
        .map_err(|error| format!("Project stage activation was rejected: {error}"))?;
    if !cancellation.is_current(request) {
        return Err("stale Project activation preparation was superseded".to_owned());
    }
    let mut target = target;
    if let Some(target) = target.as_mut() {
        let (cache_preparation, identity) = service.prepare_cache_for_activation(target);
        target.cache_identity = identity;
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
        }
    }
    if !cancellation.is_current(request) {
        return Err("stale Project activation preparation was superseded".to_owned());
    }
    Ok(target)
}

pub(super) fn install(app: &mut App) {
    app.insert_resource(ProjectStageActivationRuntime::from_environment())
        .insert_resource(ProjectActivationAuthorityRuntime::default())
        .add_systems(
            Update,
            flow::process_project_activations
                .before(usd_bevy::LiveStageSet::Project)
                .before(crate::viewport::session::spawn_when_ready),
        );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewport::api::RenderServerInterface;
    use project_protocol::{ProjectActivationCommand, ProjectActivationReply, ProjectStageTarget};
    use usd_project::{ProjectId, ProjectRoot, SceneId};
    use viewport_protocol::SessionId;
    use viewport_streaming::ProjectActivationResult as RoutedProjectActivationResult;

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

        assert!(runtime.submit(request.clone()).is_none());
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

#[cfg(test)]
#[path = "project_activation_production_tests.rs"]
mod production_tests;

#[cfg(test)]
pub(crate) use flow::apply_prepared_activation_for_test;
#[cfg(test)]
pub(crate) use production_tests::ProductionActivationWorld;

#[cfg(test)]
pub(crate) fn observe_project_activation_for_test(
    world: &mut bevy::prelude::World,
    session_id: &str,
    command: &project_protocol::ProjectActivationCommand,
) -> bool {
    world
        .resource_mut::<ProjectActivationAuthorityRuntime>()
        .0
        .observe_request(session_id, command)
}
