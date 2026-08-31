use std::{collections::HashMap, fs, path::PathBuf};

use project_protocol::{ProjectReadError, ProjectReadErrorCode, ProjectStageTarget};
use usd_project::ProjectId;

use super::ProjectApplicationService;
use crate::project::{
    cache::ProjectCacheTarget, cache_warmer::ProjectCachePreparation, scene::authoring::scene_path,
};

/// Manifest-backed semantic labels for one resolved Project stage. This
/// neutral value crosses the library/binary boundary; the viewport converts
/// it into a Bevy resource at successful activation time.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProjectStagePresentationContext {
    pub root_path: Option<String>,
    pub root_name: Option<String>,
    pub target_names: HashMap<(String, String), String>,
}

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
    pub(crate) presentation: ProjectStagePresentationContext,
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

        let presentation = presentation_context(&manifest, &target);
        Ok(Some(ProjectStageActivationTarget {
            project_id,
            target,
            project_root,
            path,
            presentation,
        }))
    }

    /// Prepare the resolved target's derived cache before the render host
    /// opens it. The wait remains on the activation preparation worker.
    pub(crate) fn prepare_cache_for_activation(
        &self,
        target: &ProjectStageActivationTarget,
    ) -> ProjectCachePreparation {
        let cache_target = match target.target {
            ProjectStageTarget::ProjectRoot(_) => ProjectCacheTarget::ProjectRoot,
            ProjectStageTarget::Scene(scene_id) => ProjectCacheTarget::Scene {
                id: scene_id.to_string(),
            },
            ProjectStageTarget::Model(model_id) => ProjectCacheTarget::Model {
                id: model_id.to_string(),
            },
        };
        self.cache_warm
            .prepare_for_activation(&target.project_root, cache_target)
    }
}

fn presentation_context(
    manifest: &usd_project::ValidatedProjectManifest,
    target: &ProjectStageTarget,
) -> ProjectStagePresentationContext {
    let root_path = match target {
        ProjectStageTarget::Model(_)
        | ProjectStageTarget::ProjectRoot(usd_project::ProjectRoot::Model(_)) => "/ModelRoot",
        _ => "/SceneRoot",
    };
    let mut context = ProjectStagePresentationContext {
        root_path: Some(root_path.to_owned()),
        root_name: None,
        target_names: Default::default(),
    };
    for scene in manifest.scenes() {
        context.target_names.insert(
            ("scene".to_owned(), scene.id.to_string()),
            scene.display_name.clone(),
        );
    }
    for model in manifest.models() {
        context.target_names.insert(
            ("model".to_owned(), model.id.to_string()),
            model.display_name.clone(),
        );
    }
    context.root_name = match target {
        ProjectStageTarget::ProjectRoot(_) => Some(manifest.raw().name.clone()),
        ProjectStageTarget::Scene(scene_id) => manifest
            .scene(*scene_id)
            .map(|entry| entry.display_name.clone()),
        ProjectStageTarget::Model(model_id) => manifest
            .model(*model_id)
            .map(|entry| entry.display_name.clone()),
    };
    context
}

fn invalid_project_data(project_id: ProjectId) -> ProjectReadError {
    ProjectReadError::Unavailable {
        project_id,
        code: ProjectReadErrorCode::InvalidProjectData,
    }
}
