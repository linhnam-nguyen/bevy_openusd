//! Read-only Project application facade.
//!
//! This is the only backend entry point needed by the M13 native host. It
//! resolves a stable ProjectId through the private workspace registry and
//! returns owned, Git-neutral protocol DTOs.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use project_protocol::{
    ProjectReadCommand, ProjectReadError, ProjectReadErrorCode, ProjectReadReply,
    ProjectReadRequest, ProjectReadResponse,
};
use usd_project::ProjectId;

use crate::project::cache_warmer::ProjectCacheWarmQueue;
use crate::project::catalog::{
    catalogue::{list_projects, unavailable_reason},
    manifest_store::ManifestStore,
    workspace_registry::{WorkspaceProjectEntry, WorkspaceRegistry},
};

use self::error::project_read_error_code;
use self::runtime_authority::NoopProjectRuntimeAuthority;

/// Read-only Project application service owned by the backend boundary.
pub struct ProjectApplicationService {
    registry: WorkspaceRegistry,
    pub(super) publication_coordinator: ProjectPublicationCoordinator,
    pub(super) stage_mutations: ProjectStageMutationQueue,
    pub(super) progress: ProjectImportProgressStore,
    pub(super) cache_warm: ProjectCacheWarmQueue,
}

/// Shared admission state for non-idempotent publication mutations.
///
/// The host may construct a fresh application service for each command, so
/// this coordinator must outlive an individual request. The map lock is held
/// only while resolving a Project-specific lock; publication work is serialized
/// by the returned lock and never by one global Project mutex.
#[derive(Clone)]
pub struct ProjectPublicationCoordinator {
    publishers: Arc<Mutex<HashMap<ProjectId, Arc<Mutex<()>>>>>,
    runtime_authority: Arc<dyn ProjectRuntimeAuthority>,
}

impl ProjectPublicationCoordinator {
    pub fn with_runtime_authority(runtime_authority: Arc<dyn ProjectRuntimeAuthority>) -> Self {
        Self {
            publishers: Arc::new(Mutex::new(HashMap::new())),
            runtime_authority,
        }
    }

    pub fn with_runtime_authority_queue() -> Self {
        Self::with_runtime_authority(Arc::new(ProjectRuntimeAuthorityQueue::default()))
    }

    pub fn publisher(&self, project_id: ProjectId) -> Arc<Mutex<()>> {
        self.publishers
            .lock()
            .expect("Project publication coordinator is not poisoned")
            .entry(project_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub(crate) fn runtime_authority_arc(&self) -> Arc<dyn ProjectRuntimeAuthority> {
        self.runtime_authority.clone()
    }
}

impl Default for ProjectPublicationCoordinator {
    fn default() -> Self {
        Self::with_runtime_authority(Arc::new(NoopProjectRuntimeAuthority))
    }
}

mod branch;
#[cfg(test)]
mod branch_projection_tests;
#[cfg(test)]
mod branch_tests;
mod commit;
mod commit_runtime;
#[cfg(test)]
mod commit_tests;
mod deletion;
#[cfg(test)]
#[path = "deletion_tests.rs"]
mod deletion_tests;
mod error;
mod export;
#[cfg(test)]
#[path = "export_portability_tests.rs"]
mod export_portability_tests;
#[cfg(test)]
mod export_tests;
mod inspection;
mod lifecycle;
mod model;
mod model_preparation;
mod progress;
mod project_registration;
mod read;
mod rename;
mod runtime_authority;
#[cfg(test)]
mod runtime_authority_owner_review5_tests;
#[cfg(test)]
mod runtime_authority_tests;
mod scene;
mod scene_adoption;
#[cfg(test)]
#[path = "scene_adoption_error_tests.rs"]
mod scene_adoption_error_tests;
pub(crate) mod scene_closure;
mod scene_lifecycle;
mod stage_activation;
mod stage_mutation;
pub use model_preparation::ProjectModelPreparationQueue;
pub use progress::ProjectImportProgressStore;
pub use runtime_authority::{
    ProjectRuntimeAuthority, ProjectRuntimeAuthorityQueue, ProjectRuntimeSnapshot,
};
pub(crate) use runtime_authority::{
    ProjectRuntimeEnvelope, ProjectRuntimeRequest, ProjectRuntimeResponse, unix_time_ms,
};
pub use scene_inspection::ProjectSceneInspectionQueue;
pub use stage_activation::{
    ActiveProjectStage, ProjectActivationAuthority, ProjectStageActivationTarget,
    ProjectStagePresentationContext,
};
#[cfg(test)]
pub use stage_activation::{ProjectStageActivationSession, ProjectStageSessionSnapshot};
pub use stage_mutation::{ProjectStageMutation, ProjectStageMutationQueue};
mod scene_inspection;
use self::read::{project_list_item, project_tree, repository_summary};
impl ProjectApplicationService {
    /// Open the host-owned workspace registry without exposing its locator.
    pub fn open(registry_path: impl Into<PathBuf>) -> Result<Self, ProjectReadError> {
        Self::open_with_publication_coordinator(
            registry_path,
            ProjectPublicationCoordinator::default(),
        )
    }

    /// Open the service with host-owned shared publication admission state.
    pub fn open_with_publication_coordinator(
        registry_path: impl Into<PathBuf>,
        publication_coordinator: ProjectPublicationCoordinator,
    ) -> Result<Self, ProjectReadError> {
        Self::open_with_project_state(
            registry_path,
            publication_coordinator,
            ProjectStageMutationQueue::default(),
        )
    }

    /// Open the service with all host-owned shared Project state.
    pub fn open_with_project_state(
        registry_path: impl Into<PathBuf>,
        publication_coordinator: ProjectPublicationCoordinator,
        stage_mutations: ProjectStageMutationQueue,
    ) -> Result<Self, ProjectReadError> {
        Self::open_with_project_state_and_progress(
            registry_path,
            publication_coordinator,
            stage_mutations,
            ProjectImportProgressStore::default(),
        )
    }

    /// Open the service with all host-owned shared Project state, including
    /// the progress status observed by the native host.
    pub fn open_with_project_state_and_progress(
        registry_path: impl Into<PathBuf>,
        publication_coordinator: ProjectPublicationCoordinator,
        stage_mutations: ProjectStageMutationQueue,
        progress: ProjectImportProgressStore,
    ) -> Result<Self, ProjectReadError> {
        let registry = WorkspaceRegistry::load(registry_path).map_err(|_| {
            ProjectReadError::HostUnavailable {
                code: ProjectReadErrorCode::RegistryUnavailable,
            }
        })?;
        migrate_registered_project_roots(&registry);
        Ok(Self {
            registry,
            publication_coordinator,
            stage_mutations,
            progress,
            cache_warm: ProjectCacheWarmQueue::default(),
        })
    }

    /// Open with a shared LiveStage handoff queue and default publication
    /// admission. This is useful for hosts that already own stage activation.
    pub fn open_with_stage_mutation_queue(
        registry_path: impl Into<PathBuf>,
        stage_mutations: ProjectStageMutationQueue,
    ) -> Result<Self, ProjectReadError> {
        Self::open_with_project_state(
            registry_path,
            ProjectPublicationCoordinator::default(),
            stage_mutations,
        )
    }

    /// Execute one versioned read command and return a typed reply envelope.
    pub fn execute(&self, command: ProjectReadCommand) -> ProjectReadReply {
        match command.validate() {
            Ok(()) => match self.read(&command.request) {
                Ok(response) => ProjectReadReply::success(response),
                Err(error) => ProjectReadReply::failure(error),
            },
            Err(error) => ProjectReadReply::failure(error),
        }
    }

    fn read(&self, request: &ProjectReadRequest) -> Result<ProjectReadResponse, ProjectReadError> {
        match request {
            ProjectReadRequest::ListProjects => Ok(ProjectReadResponse::Projects(
                list_projects(&self.registry)
                    .into_iter()
                    .map(project_list_item)
                    .collect(),
            )),
            ProjectReadRequest::GetProjectTree(project_id) => {
                let (entry, manifest) = self.validated_project(*project_id)?;
                let (nodes, counts) = project_tree(entry.repository_locator(), &manifest)?;
                Ok(ProjectReadResponse::ProjectTree {
                    project_id: *project_id,
                    nodes,
                    counts,
                })
            }
            ProjectReadRequest::GetProjectRepositorySummary(project_id) => {
                let (entry, _) = self.validated_project(*project_id)?;
                Ok(ProjectReadResponse::RepositorySummary {
                    project_id: *project_id,
                    repository: repository_summary(*project_id, entry.repository_locator())?,
                })
            }
        }
    }

    fn validated_project(
        &self,
        project_id: ProjectId,
    ) -> Result<
        (
            &WorkspaceProjectEntry,
            usd_project::ValidatedProjectManifest,
        ),
        ProjectReadError,
    > {
        let entry = self
            .registry
            .get(project_id)
            .ok_or(ProjectReadError::NotFound { project_id })?;
        let manifest = ManifestStore::read_validated(entry.repository_locator()).map_err(|_| {
            ProjectReadError::Unavailable {
                project_id,
                code: project_read_error_code(unavailable_reason(entry)),
            }
        })?;
        if manifest.raw().project_id != entry.content_project_id() {
            return Err(ProjectReadError::Unavailable {
                project_id,
                code: ProjectReadErrorCode::RegistryIdentityMismatch,
            });
        }
        Ok((entry, manifest))
    }
}

fn migrate_registered_project_roots(registry: &WorkspaceRegistry) {
    for entry in registry.entries() {
        let project_root = entry.repository_locator();
        let Ok(manifest) = ManifestStore::read_validated(project_root) else {
            continue;
        };
        if let Err(error) =
            crate::project::link::migrate_linked_source_provenance(project_root, manifest.raw())
        {
            log::warn!(
                "Linked Scene provenance migration skipped for {}: {error:#}",
                project_root.display()
            );
        }
        if let Err(error) = crate::project::scene::root::ensure_protected_root_scene_atomic(
            project_root,
            manifest.raw(),
        ) {
            log::warn!(
                "Project Root Scene migration skipped for {}: {error:#}",
                project_root.display()
            );
        }
    }
}

#[cfg(test)]
#[path = "repository_summary_tests.rs"]
mod repository_summary_tests;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "m19_tests.rs"]
mod m19_tests;

#[cfg(test)]
mod or8_m2;
