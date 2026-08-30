use std::{
    env, fs,
    path::{Path, PathBuf},
};

use project_protocol::{
    ProjectCommitRequest, ProjectCommitResponse, ProjectExportSceneRequest, ProjectInspection,
    ProjectSceneExportResponse, ProjectWriteError, ProjectWriteErrorCode,
};
use usd_project::ProjectSummary;
use uuid::Uuid;

use super::ProjectApplicationService;
use crate::project::catalog::manifest_store::ManifestStore;

impl ProjectApplicationService {
    pub fn commit(
        &mut self,
        request: ProjectCommitRequest,
    ) -> Result<ProjectCommitResponse, ProjectWriteError> {
        super::commit::commit(self, request)
    }

    pub fn export_scene(
        &mut self,
        request: ProjectExportSceneRequest,
        destination: &Path,
    ) -> Result<ProjectSceneExportResponse, ProjectWriteError> {
        super::export::export_scene(self, request, destination)
    }

    pub fn inspect_project(
        &self,
        project_root: &Path,
    ) -> Result<ProjectInspection, ProjectWriteError> {
        super::inspection::inspect_project(project_root)
    }

    pub fn create_project(
        &mut self,
        parent: &Path,
        name: &str,
    ) -> Result<ProjectSummary, ProjectWriteError> {
        super::project_registration::create_project(self, parent, name)
    }

    pub fn import_project(
        &mut self,
        project_root: &Path,
        expected: &ProjectInspection,
    ) -> Result<ProjectSummary, ProjectWriteError> {
        super::project_registration::import_project(self, project_root, expected)
    }

    pub fn create_scene(
        &mut self,
        project_id: usd_project::ProjectId,
        target: project_protocol::ProjectWriteTarget,
        name: &str,
    ) -> Result<project_protocol::ProjectSceneWriteResponse, ProjectWriteError> {
        super::scene::create_scene(self, project_id, target, name)
    }

    pub fn rename(
        &mut self,
        project_id: usd_project::ProjectId,
        target: project_protocol::ProjectWriteTarget,
        name: &str,
    ) -> Result<project_protocol::ProjectRenameResponse, ProjectWriteError> {
        super::rename::rename(self, project_id, target, name)
    }

    pub fn remove_project(
        &mut self,
        project_id: usd_project::ProjectId,
    ) -> Result<project_protocol::ProjectLifecycleResponse, ProjectWriteError> {
        if self.registry.get(project_id).is_none() {
            return Err(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::ProjectNotFound,
            });
        }
        self.registry
            .remove(project_id)
            .map_err(|_| ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::ProjectRemoveFailed,
            })?;
        Ok(project_protocol::ProjectLifecycleResponse { project_id })
    }

    pub fn delete_project(
        &mut self,
        project_id: usd_project::ProjectId,
    ) -> Result<project_protocol::ProjectLifecycleResponse, ProjectWriteError> {
        let project_root = self.validated_delete_root(project_id)?;
        let parent = project_root.parent().ok_or(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::ProtectedProjectPath,
        })?;
        let name = project_root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::ProtectedProjectPath,
            })?;
        let tombstone = parent.join(format!(".{name}.usdhub-delete-{}", Uuid::new_v4()));
        fs::rename(&project_root, &tombstone).map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::ProjectDeleteFailed,
        })?;

        if self.registry.remove(project_id).is_err() {
            let _ = fs::rename(&tombstone, &project_root);
            return Err(ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::ProjectDeleteFailed,
            });
        }
        if fs::remove_dir_all(&tombstone).is_err() {
            return Err(ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::ProjectDeleteCleanupFailed,
            });
        }
        Ok(project_protocol::ProjectLifecycleResponse { project_id })
    }

    pub fn adopt_scene(
        &mut self,
        project_id: usd_project::ProjectId,
        target: project_protocol::ProjectWriteTarget,
        source: &Path,
        inspection: &usd_project::CompositionInspection,
        name: String,
        operation_id: String,
        generation: u64,
        placement: project_protocol::PlacementSpec,
    ) -> Result<project_protocol::ProjectSceneAdoptionResponse, ProjectWriteError> {
        super::scene_adoption::adopt_scene(
            self,
            project_id,
            target,
            source,
            inspection,
            name,
            operation_id,
            generation,
            placement,
        )
    }

    pub fn link_scene(
        &mut self,
        project_id: usd_project::ProjectId,
        target: project_protocol::ProjectWriteTarget,
        source: &Path,
        inspection: &usd_project::CompositionInspection,
        name: String,
        operation_id: String,
        generation: u64,
        placement: project_protocol::PlacementSpec,
    ) -> Result<project_protocol::ProjectSceneAdoptionResponse, ProjectWriteError> {
        super::scene_adoption::link_scene(
            self,
            project_id,
            target,
            source,
            inspection,
            name,
            operation_id,
            generation,
            placement,
        )
    }

    pub fn sync_linked_scene(
        &mut self,
        project_id: usd_project::ProjectId,
        scene_id: usd_project::SceneId,
        source: &Path,
        inspection: &usd_project::CompositionInspection,
        name: String,
        operation_id: String,
        generation: u64,
    ) -> Result<project_protocol::ProjectSceneAdoptionResponse, ProjectWriteError> {
        super::scene_adoption::sync_linked_scene(
            self,
            project_id,
            scene_id,
            source,
            inspection,
            name,
            operation_id,
            generation,
        )
    }

    pub fn publish_model(
        &mut self,
        preparation: &super::ProjectModelPreparationQueue,
        project_id: usd_project::ProjectId,
        target: project_protocol::ProjectWriteTarget,
        source: &std::path::Path,
        operation_id: String,
        generation: u64,
        placement: project_protocol::PlacementSpec,
    ) -> Result<project_protocol::ProjectModelWriteResponse, ProjectWriteError> {
        super::model::publish_model(
            self,
            preparation,
            project_id,
            target,
            source,
            operation_id,
            generation,
            placement,
        )
    }

    pub fn delete_model(
        &mut self,
        request: project_protocol::ProjectDeleteModelRequest,
    ) -> Result<project_protocol::ProjectModelLifecycleResponse, ProjectWriteError> {
        super::deletion::delete_model(self, request)
    }
}

impl ProjectApplicationService {
    fn validated_delete_root(
        &self,
        project_id: usd_project::ProjectId,
    ) -> Result<PathBuf, ProjectWriteError> {
        let entry = self
            .registry
            .get(project_id)
            .ok_or(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::ProjectNotFound,
            })?;
        let root = fs::canonicalize(entry.repository_locator()).map_err(|_| {
            ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::ProjectDeleteFailed,
            }
        })?;
        if !root.is_dir()
            || root == Path::new("/")
            || env::var_os("HOME")
                .is_some_and(|home| fs::canonicalize(home).is_ok_and(|home| home == root))
        {
            return Err(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::ProtectedProjectPath,
            });
        }
        let registry_path = fs::canonicalize(self.registry.registry_path()).map_err(|_| {
            ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::ProjectDeleteFailed,
            }
        })?;
        if root == registry_path
            || root == registry_path.parent().unwrap_or_else(|| Path::new("/"))
            || registry_path.starts_with(&root)
        {
            return Err(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::ProtectedProjectPath,
            });
        }
        usd_git::Repository::open(&root).map_err(|_| ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::ProjectDeleteFailed,
        })?;
        let manifest =
            ManifestStore::read_validated(&root).map_err(|_| ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::ProjectDeleteFailed,
            })?;
        if manifest.raw().project_id != project_id {
            return Err(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::ProjectDeleteFailed,
            });
        }
        Ok(root)
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "lifecycle_import_tests.rs"]
mod import_tests;

#[cfg(test)]
#[path = "lifecycle_m15_tests.rs"]
mod m15_tests;
