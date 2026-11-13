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

#[path = "cache_presentation_render.rs"]
mod cache_presentation_render;
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

/// Cache-first activation continuation. The bootstrap is committed in one
/// update, then the canonical Stage is opened in a later update so Bevy gets
/// a render opportunity before the potentially blocking OpenUSD boundary.
#[derive(Resource)]
pub(super) struct PendingCanonicalStageActivation {
    pub(super) request: ProjectActivationRequest,
    pub(super) target: crate::project::service::ProjectStageActivationTarget,
    pub(super) wait_for_next_update: bool,
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
        let (cache_preparation, identity, scene_cache) =
            service.prepare_cache_for_activation(target);
        target.cache_identity = identity;
        target.scene_cache = scene_cache;
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
    cache_presentation_render::install(app);
    app.insert_resource(ProjectStageActivationRuntime::from_environment())
        .insert_resource(ProjectActivationAuthorityRuntime::default())
        .add_systems(
            Update,
            flow::process_project_activations
                .before(usd_bevy::LiveStageSet::Project)
                .before(crate::viewport::session::spawn_when_ready),
        )
        .add_systems(
            Update,
            flow::continue_deferred_stage_activation
                .after(flow::process_project_activations)
                .before(usd_bevy::LiveStageSet::Project),
        );
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/app/project_activation_inline_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../tests/unit/viewport/app/project_activation_cache_tests.rs"]
mod cache_tests;

#[cfg(test)]
#[path = "../../../tests/unit/viewport/app/project_activation_production_tests.rs"]
mod production_tests;

#[cfg(test)]
pub(crate) use flow::apply_prepared_activation_for_test;
#[cfg(test)]
pub(crate) use flow::continue_deferred_stage_activation_for_test;
#[cfg(test)]
pub(crate) use production_tests::ProductionActivationWorld;

#[cfg(test)]
#[path = "../../../tests/unit/viewport/app/project_activation_test_support.rs"]
mod test_support;
#[cfg(test)]
pub(crate) use test_support::observe_project_activation_for_test;
