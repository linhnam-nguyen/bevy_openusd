use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use openusd::usd::Stage;
use project_protocol::{
    ProjectActivationCommand, ProjectReadError, ProjectReadErrorCode, ProjectStageTarget,
};
use usd_project::ProjectId;

use super::ProjectApplicationService;
use crate::project::{
    cache::{ProjectCacheIdentity, ProjectCacheTarget},
    cache_warmer::{ProjectCachePreparation, ProjectCacheWarmQueue},
    scene::authoring::scene_path,
};

#[path = "stage_archive.rs"]
mod stage_archive;

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
    pub(crate) archive_paths: Vec<PathBuf>,
    pub(crate) cache_identity: Option<ProjectCacheIdentity>,
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
        let target_started = Instant::now();
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

        let archive_started = Instant::now();
        let archive_paths =
            stage_archive::for_target(&project_root, manifest.raw(), &target, &path)
                .unwrap_or_else(|error| {
                    log::warn!(
                        "Project activation archive discovery failed for {}: {error:#}",
                        path.display()
                    );
                    Vec::new()
                });
        log::debug!(
            "[project-loading] archive_discovery_ms={:.3} packages={} target={}",
            archive_started.elapsed().as_secs_f64() * 1_000.0,
            archive_paths.len(),
            path.display()
        );
        log::debug!(
            "[project-loading] target_resolution_ms={:.3} target={}",
            target_started.elapsed().as_secs_f64() * 1_000.0,
            path.display()
        );
        let presentation = presentation_context(&manifest, &target);
        Ok(Some(ProjectStageActivationTarget {
            project_id,
            target,
            project_root,
            path,
            archive_paths,
            cache_identity: None,
            presentation,
        }))
    }

    /// Probe the resolved target's derived cache on the preparation worker.
    /// Cache warming is advisory; canonical Stage activation never waits for it.
    pub(crate) fn prepare_cache_for_activation(
        &self,
        target: &ProjectStageActivationTarget,
    ) -> (ProjectCachePreparation, Option<ProjectCacheIdentity>) {
        let cache_target = match target.target {
            ProjectStageTarget::ProjectRoot(_) => ProjectCacheTarget::ProjectRoot,
            ProjectStageTarget::Scene(scene_id) => ProjectCacheTarget::Scene {
                id: scene_id.to_string(),
            },
            ProjectStageTarget::Model(model_id) => ProjectCacheTarget::Model {
                id: model_id.to_string(),
            },
        };
        let identity_started = Instant::now();
        let identity = match ProjectCacheIdentity::for_project(
            &target.project_root,
            cache_target,
            viewport_protocol::RuntimeProfile::NativeMedium,
            crate::project::cache_hydration::default_project_cache_config_hash(),
        ) {
            Ok(identity) => identity,
            Err(error) => {
                log::warn!(
                    "Project cache activation identity could not be established for {}: {error:#}",
                    target.project_root.display()
                );
                return (ProjectCachePreparation::FallbackRequired, None);
            }
        };
        log::debug!(
            "[project-loading] cache_identity_ms={:.3} target={}",
            identity_started.elapsed().as_secs_f64() * 1_000.0,
            identity.target.key()
        );
        let probe_started = Instant::now();
        let state = ProjectCacheWarmQueue::probe_for_activation(
            &crate::project::cache::ProjectCacheStore::new(&target.project_root),
            &identity,
        );
        log::debug!(
            "[project-loading] cache_descriptor_probe_ms={:.3} state={state:?} target={}",
            probe_started.elapsed().as_secs_f64() * 1_000.0,
            identity.target.key()
        );
        (state, Some(identity))
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

/// Production candidate for the Project stage/session boundary.
///
/// The candidate owns one opened and composition-validated OpenUSD Stage. The
/// Bevy lifecycle remains the sole producer of stage-derived hierarchy and
/// semantic/BIM resources after installation.
pub struct ProjectStageActivation {
    stage: Stage,
}

impl ProjectStageActivation {
    pub fn open(
        command: &ProjectActivationCommand,
        target: ProjectStageActivationTarget,
    ) -> Result<Self, String> {
        if target.project_id != command.project_id || target.target != command.target {
            return Err("activation target does not match its command".to_owned());
        }
        let stage_started = Instant::now();
        let stage = open_activation_stage(&target.path)?;
        log::debug!(
            "[project-loading] stage_open_ms={:.3} target={}",
            stage_started.elapsed().as_secs_f64() * 1_000.0,
            target.path.display()
        );
        if !stage.composition_errors().is_empty() {
            return Err(format!(
                "activated Stage has composition errors: {:?}",
                stage.composition_errors()
            ));
        }
        Ok(Self { stage })
    }

    pub fn into_stage(self) -> Stage {
        self.stage
    }
}

fn open_activation_stage(path: &Path) -> Result<Stage, String> {
    Stage::open(&path.to_string_lossy()).map_err(|error| format!("open activated Stage: {error}"))
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
