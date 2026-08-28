use std::{fs, path::PathBuf};

use project_protocol::{ProjectReadError, ProjectReadErrorCode};
use usd_project::{ProjectId, ProjectRoot};

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
    pub root: ProjectRoot,
    pub path: PathBuf,
}

impl ProjectApplicationService {
    /// Resolve the canonical stage file for the registered Project root.
    ///
    /// Registry identity and manifest/root membership are validated before a
    /// path is returned. Empty Projects intentionally return `None` because
    /// they have no stage to open.
    pub fn resolve_stage_activation(
        &self,
        project_id: ProjectId,
        root: ProjectRoot,
    ) -> Result<Option<ProjectStageActivationTarget>, ProjectReadError> {
        let (entry, manifest) = self.validated_project(project_id)?;
        if manifest.raw().root != root {
            return Err(ProjectReadError::Unavailable {
                project_id,
                code: ProjectReadErrorCode::InvalidProjectData,
            });
        }

        let path = match root {
            ProjectRoot::Empty => return Ok(None),
            ProjectRoot::Scene(scene_id) => scene_path(entry.repository_locator(), scene_id),
            ProjectRoot::Model(model_id) => crate::project::model_wrapper::model_wrapper_path(
                entry.repository_locator(),
                model_id,
            ),
        };
        let path = fs::canonicalize(&path).map_err(|_| ProjectReadError::Unavailable {
            project_id,
            code: ProjectReadErrorCode::InvalidProjectData,
        })?;
        if !path.is_file() {
            return Err(ProjectReadError::Unavailable {
                project_id,
                code: ProjectReadErrorCode::InvalidProjectData,
            });
        }

        Ok(Some(ProjectStageActivationTarget {
            project_id,
            root,
            path,
        }))
    }
}
