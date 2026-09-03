use std::{collections::HashMap, fs, path::PathBuf};

#[cfg(test)]
use openusd::usd::{InitialLoadSet, PrimPredicate, Stage};
use project_protocol::{
    ProjectActivationCommand, ProjectReadError, ProjectReadErrorCode, ProjectStageTarget,
};
#[cfg(test)]
use usd_model::SnapshotSource;
use usd_project::ProjectId;
#[cfg(test)]
use usd_semantic::{SemanticConfig, SemanticExtractor};

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

/// Stable identity of the Project stage currently owned by one render host.
///
/// The resolved path is deliberately absent: path resolution remains private
/// to the host, while the active identity is safe to compare with protocol
/// commands and derived read-model generations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveProjectStage {
    pub project_id: ProjectId,
    pub target: ProjectStageTarget,
    pub generation: u64,
}

/// Main-thread admission and completion authority for Project activations.
///
/// A preparation worker may finish an older request after a newer request was
/// admitted. The exact command remains the completion key, so such a result
/// can never replace the latest active Project stage.
#[derive(Clone, Debug, Default)]
pub struct ProjectActivationAuthority {
    latest_by_session: HashMap<String, ProjectActivationCommand>,
    active: Option<ActiveProjectStage>,
}

impl ProjectActivationAuthority {
    /// Records a newer valid request for a transport session.
    pub fn observe_request(
        &mut self,
        session_id: &str,
        command: &ProjectActivationCommand,
    ) -> bool {
        if session_id.trim().is_empty() || command.validate().is_err() {
            return false;
        }
        let is_newer = self
            .latest_by_session
            .get(session_id)
            .is_none_or(|latest| command.generation > latest.generation);
        if is_newer {
            self.latest_by_session
                .insert(session_id.to_owned(), command.clone());
        }
        is_newer
    }

    /// Checks the exact request that is currently allowed to commit.
    pub fn is_current(&self, session_id: &str, command: &ProjectActivationCommand) -> bool {
        self.latest_by_session
            .get(session_id)
            .is_some_and(|latest| latest == command)
    }

    /// Commits an activation only if it still belongs to the latest request.
    pub fn commit(&mut self, session_id: &str, command: &ProjectActivationCommand) -> bool {
        if !self.is_current(session_id, command) {
            return false;
        }
        self.active = Some(ActiveProjectStage {
            project_id: command.project_id,
            target: command.target.clone(),
            generation: command.generation,
        });
        true
    }

    pub fn active(&self) -> Option<&ActiveProjectStage> {
        self.active.as_ref()
    }
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

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectStageSessionSnapshot {
    pub project_id: ProjectId,
    pub target: ProjectStageTarget,
    pub generation: u64,
    pub stage_path: PathBuf,
    pub hierarchy_paths: Vec<String>,
    pub bim_snapshot_id: String,
    pub bim_entity_paths: Vec<String>,
}

/// Deterministic seam for the real Project stage/session boundary.
///
/// It resolves and opens the canonical Stage, extracts the same semantic
/// snapshot used by the viewport BIM provider, and commits only the newest
/// request. No GPU, window, or transport is needed to prove stale completion
/// handling and latest-scene read-model ownership.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct ProjectStageActivationSession {
    authority: ProjectActivationAuthority,
    active: Option<ProjectStageSessionSnapshot>,
}

#[cfg(test)]
impl ProjectStageActivationSession {
    pub fn observe_request(
        &mut self,
        session_id: &str,
        command: &ProjectActivationCommand,
    ) -> bool {
        self.authority.observe_request(session_id, command)
    }

    pub fn complete(
        &mut self,
        session_id: &str,
        command: &ProjectActivationCommand,
        target: ProjectStageActivationTarget,
    ) -> Result<ProjectStageSessionSnapshot, String> {
        if !self.authority.is_current(session_id, command) {
            return Err("stale Project activation completion was rejected".to_owned());
        }
        if target.project_id != command.project_id || target.target != command.target {
            return Err("activation target does not match its command".to_owned());
        }
        let stage = open_activation_stage(&target.path)?;
        let mut hierarchy_paths = Vec::new();
        stage
            .traverse(PrimPredicate::DEFAULT, |path| {
                hierarchy_paths.push(path.as_str().to_owned())
            })
            .map_err(|error| format!("traverse activated Stage: {error}"))?;
        if !stage.composition_errors().is_empty() {
            return Err(format!(
                "activated Stage has composition errors: {:?}",
                stage.composition_errors()
            ));
        }
        let semantic = SemanticExtractor::new(SemanticConfig::for_nvidia_revit_connector())
            .extract(
                &stage,
                SnapshotSource::Working {
                    session: session_id.to_owned(),
                    live_revision: command.generation,
                },
            )
            .map_err(|error| format!("extract activated Stage semantics: {error}"))?;
        let mut bim_entity_paths = semantic
            .entities
            .values()
            .filter(|entity| entity.semantic.is_bim_entity())
            .map(|entity| entity.prim_path.clone())
            .collect::<Vec<_>>();
        bim_entity_paths.sort();
        let snapshot = ProjectStageSessionSnapshot {
            project_id: command.project_id,
            target: command.target.clone(),
            generation: command.generation,
            stage_path: target.path,
            hierarchy_paths,
            bim_snapshot_id: semantic.snapshot_id.0,
            bim_entity_paths,
        };
        if !self.authority.commit(session_id, command) {
            return Err("stale Project activation completion was rejected".to_owned());
        }
        self.active = Some(snapshot.clone());
        Ok(snapshot)
    }

    pub fn active(&self) -> Option<&ProjectStageSessionSnapshot> {
        self.active.as_ref()
    }
}

#[cfg(test)]
fn open_activation_stage(path: &std::path::Path) -> Result<Stage, String> {
    Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(path.to_string_lossy().as_ref())
        .map_err(|error| format!("open activated Stage: {error}"))
}

#[cfg(test)]
mod authority_tests {
    use super::*;
    use usd_project::SceneId;

    fn command(generation: u64) -> ProjectActivationCommand {
        ProjectActivationCommand::new(
            format!("authority-{generation}"),
            generation,
            ProjectId::new_v4(),
            ProjectStageTarget::Scene(SceneId::new_v4()),
        )
    }

    #[test]
    fn stale_completion_cannot_replace_latest_active_identity() {
        let mut authority = ProjectActivationAuthority::default();
        let first = command(1);
        let second = command(2);

        assert!(authority.observe_request("session", &first));
        assert!(authority.commit("session", &first));
        assert!(authority.observe_request("session", &second));
        assert!(!authority.commit("session", &first));
        assert!(authority.commit("session", &second));
        assert_eq!(
            authority.active(),
            Some(&ActiveProjectStage {
                project_id: second.project_id,
                target: second.target,
                generation: 2,
            })
        );
    }
}
