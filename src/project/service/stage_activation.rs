use std::{fs, path::PathBuf};

use project_protocol::{ProjectReadError, ProjectReadErrorCode, ProjectStageTarget};
use usd_project::ProjectId;

use super::ProjectApplicationService;
use crate::project::scene::authoring::scene_path;

/// Backend-only canonical stage target resolved from a Project identity.
///
/// This type never crosses the frontend or Project wire protocol. It exists
/// so the render host can hand the resolved path to the existing stage-open
/// lifecycle without teaching that lifecycle about registries or manifests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectStageActivationTarget {
    pub project_id: ProjectId,
    pub target: ProjectStageTarget,
    pub project_root: PathBuf,
    pub path: PathBuf,
}

impl ProjectApplicationService {
    /// Resolve the canonical stage file for a registered Project stage target.
    ///
    /// Registry identity and manifest/root membership are validated before a
    /// path is returned. Empty Projects intentionally return `None` because
    /// they have no stage to open.
    pub fn resolve_stage_activation(
        &self,
        project_id: ProjectId,
        target: ProjectStageTarget,
    ) -> Result<Option<ProjectStageActivationTarget>, ProjectReadError> {
        let (entry, manifest) = self.validated_project(project_id)?;
        let project_root = fs::canonicalize(entry.repository_locator())
            .map_err(|_| invalid_project_data(project_id))?;
        let path = match &target {
            ProjectStageTarget::ProjectRoot(root) => {
                if &manifest.raw().root != root {
                    return Err(invalid_project_data(project_id));
                }
                match root {
                    usd_project::ProjectRoot::Empty => return Ok(None),
                    usd_project::ProjectRoot::Scene(scene_id) => {
                        scene_path(entry.repository_locator(), *scene_id)
                    }
                    usd_project::ProjectRoot::Model(model_id) => {
                        crate::project::model_wrapper::model_wrapper_path(
                            entry.repository_locator(),
                            *model_id,
                        )
                    }
                }
            }
            ProjectStageTarget::Scene(scene_id) => {
                if manifest.scene(*scene_id).is_none() {
                    return Err(invalid_project_data(project_id));
                }
                scene_path(entry.repository_locator(), *scene_id)
            }
            ProjectStageTarget::Model(model_id) => {
                if manifest.model(*model_id).is_none() {
                    return Err(invalid_project_data(project_id));
                }
                crate::project::model_wrapper::model_wrapper_path(
                    entry.repository_locator(),
                    *model_id,
                )
            }
        };
        let path = fs::canonicalize(&path).map_err(|_| invalid_project_data(project_id))?;
        if !path.is_file() {
            return Err(invalid_project_data(project_id));
        }

        Ok(Some(ProjectStageActivationTarget {
            project_id,
            target,
            project_root,
            path,
        }))
    }
}

fn invalid_project_data(project_id: ProjectId) -> ProjectReadError {
    ProjectReadError::Unavailable {
        project_id,
        code: ProjectReadErrorCode::InvalidProjectData,
    }
}
