//! Application-to-LiveStage authority boundary for Project mutations.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use project_protocol::ProjectWriteError;
use usd_project::{ModelId, ProjectId, SceneId, SceneMemberId};

/// A typed Project mutation waiting for the active-stage owner.
///
/// This DTO contains only stable Project identities. The OpenUSD `Stage` and
/// Bevy entities remain owned by the active-stage thread.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectStageMutation {
    CreateScene {
        project_id: ProjectId,
        scene_id: SceneId,
        parent_scene_id: Option<SceneId>,
        placement_id: Option<SceneMemberId>,
    },
    AdoptScene {
        project_id: ProjectId,
        scene_id: SceneId,
        parent_scene_id: Option<SceneId>,
        placement_id: Option<SceneMemberId>,
    },
    PublishModel {
        project_id: ProjectId,
        model_id: ModelId,
        parent_scene_id: Option<SceneId>,
        placement_id: Option<SceneMemberId>,
    },
}

const STAGE_MUTATION_CAPACITY: usize = 128;

/// Shared host queue connecting request-scoped Project services to the active
/// LiveStage owner. A queue entry is applied only when its Project is active.
#[derive(Clone, Default)]
pub struct ProjectStageMutationQueue {
    pending: Arc<Mutex<VecDeque<ProjectStageMutation>>>,
}

impl ProjectStageMutationQueue {
    pub fn submit(&self, mutation: ProjectStageMutation) -> Result<(), ProjectWriteError> {
        let mut pending = self
            .pending
            .lock()
            .expect("Project stage mutation queue is not poisoned");
        if pending.len() >= STAGE_MUTATION_CAPACITY {
            return Err(ProjectWriteError::Failed {
                code: project_protocol::ProjectWriteErrorCode::Busy,
            });
        }
        pending.push_back(mutation);
        Ok(())
    }

    /// Apply queued mutations for the currently active Project on the
    /// LiveStage owner thread. Other Project entries remain queued.
    pub fn apply_for_project(
        &self,
        live: &usd_bevy::LiveStage,
        active_project_id: ProjectId,
    ) -> Result<usize, ProjectWriteError> {
        let selected = {
            let mut pending = self
                .pending
                .lock()
                .expect("Project stage mutation queue is not poisoned");
            let mut selected = Vec::new();
            let mut retained = VecDeque::with_capacity(pending.len());
            while let Some(mutation) = pending.pop_front() {
                if mutation.project_id() == active_project_id {
                    selected.push(mutation);
                } else {
                    retained.push_back(mutation);
                }
            }
            *pending = retained;
            selected
        };

        let mut applied = 0;
        for mutation in selected {
            apply_mutation(live, &mutation)?;
            applied += 1;
        }
        Ok(applied)
    }

    #[cfg(test)]
    fn pending_len(&self) -> usize {
        self.pending
            .lock()
            .expect("Project stage mutation queue is not poisoned")
            .len()
    }
}

impl ProjectStageMutation {
    fn project_id(&self) -> ProjectId {
        match self {
            Self::CreateScene { project_id, .. }
            | Self::AdoptScene { project_id, .. }
            | Self::PublishModel { project_id, .. } => *project_id,
        }
    }
}

fn apply_mutation(
    live: &usd_bevy::LiveStage,
    mutation: &ProjectStageMutation,
) -> Result<(), ProjectWriteError> {
    let (content_path, placement_path) = match mutation {
        ProjectStageMutation::CreateScene {
            project_id,
            scene_id,
            parent_scene_id,
            placement_id,
        }
        | ProjectStageMutation::AdoptScene {
            project_id,
            scene_id,
            parent_scene_id,
            placement_id,
        } => (
            format!(
                "{}/scene_{}",
                scene_parent_path(*project_id, *parent_scene_id),
                scene_id.as_uuid().simple()
            ),
            placement_id.map(|id| {
                format!(
                    "{}/placement_{}",
                    scene_parent_path(*project_id, *parent_scene_id),
                    id.as_uuid().simple()
                )
            }),
        ),
        ProjectStageMutation::PublishModel {
            project_id,
            model_id,
            parent_scene_id,
            placement_id,
        } => (
            format!(
                "{}/model_{}",
                scene_parent_path(*project_id, *parent_scene_id),
                model_id.as_uuid().simple()
            ),
            placement_id.map(|id| {
                format!(
                    "{}/placement_{}",
                    scene_parent_path(*project_id, *parent_scene_id),
                    id.as_uuid().simple()
                )
            }),
        ),
    };

    usd_bevy::define_prim(&live.stage, &content_path, "Xform").map_err(|_| {
        ProjectWriteError::Failed {
            code: project_protocol::ProjectWriteErrorCode::FilesystemFailure,
        }
    })?;
    if let Some(placement_path) = placement_path {
        usd_bevy::define_prim(&live.stage, &placement_path, "Xform").map_err(|_| {
            ProjectWriteError::Failed {
                code: project_protocol::ProjectWriteErrorCode::FilesystemFailure,
            }
        })?;
    }
    Ok(())
}

fn scene_parent_path(project_id: ProjectId, parent_scene_id: Option<SceneId>) -> String {
    let project_path = format!("/__usdhub/project_{}", project_id.as_uuid().simple());
    parent_scene_id.map_or(project_path.clone(), |scene_id| {
        format!("{project_path}/scene_{}", scene_id.as_uuid().simple())
    })
}

#[cfg(test)]
mod tests {
    use openusd::usd::Stage;

    use super::*;

    #[test]
    fn actual_project_mutations_reach_one_live_stage_change_batch() {
        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("projects");
        std::fs::create_dir(&parent).unwrap();
        let queue = ProjectStageMutationQueue::default();
        let mut service = super::super::ProjectApplicationService::open_with_stage_mutation_queue(
            directory.path().join("workspace.json"),
            queue.clone(),
        )
        .unwrap();
        let project = service.create_project(&parent, "Project").unwrap();
        let scene = service
            .create_scene(
                project.id,
                project_protocol::ProjectWriteTarget::Project(project.id),
                "Scene",
            )
            .unwrap();
        let source = directory.path().join("assembly.usda");
        std::fs::write(
            &source,
            "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (kind = \"assembly\") {}\n",
        )
        .unwrap();
        let inspection = crate::project::scene::inspection::inspect_composition(&source).unwrap();
        let adopted = service
            .adopt_scene(
                project.id,
                project_protocol::ProjectWriteTarget::Scene(scene.scene_id),
                &source,
                &inspection,
                "adopt".to_owned(),
                1,
            )
            .unwrap();
        let preparations = super::super::ProjectModelPreparationQueue::default();
        preparations.prepare("model".to_owned(), 1, source.clone());
        let model = service
            .publish_model(
                &preparations,
                project.id,
                project_protocol::ProjectWriteTarget::Scene(scene.scene_id),
                &source,
                "model".to_owned(),
                1,
            )
            .unwrap();

        let live = usd_bevy::LiveStage::new(
            Stage::builder()
                .in_memory("project-active-stage.usda")
                .unwrap(),
        );
        assert_eq!(queue.apply_for_project(&live, project.id).unwrap(), 3);
        let batch = live.drain_change_batch().expect("Project mutations batch");
        assert_eq!(batch.revision, usd_bevy::LiveRevision(1));
        assert!(batch.has_resync());
        let contains_path = |needle: String| {
            batch.changes.iter().any(|change| {
                change
                    .resynced
                    .iter()
                    .chain(change.changed_info.iter())
                    .any(|path| path.contains(&needle))
            })
        };
        assert!(contains_path(scene.scene_id.as_uuid().simple().to_string()));
        assert!(contains_path(
            adopted.scene_id.as_uuid().simple().to_string()
        ));
        assert!(contains_path(model.model_id.as_uuid().simple().to_string()));
        assert!(live.drain_change_batch().is_none());
    }

    #[test]
    fn inactive_project_mutations_remain_queued() {
        let queue = ProjectStageMutationQueue::default();
        let first = ProjectId::new_v4();
        let second = ProjectId::new_v4();
        queue
            .submit(ProjectStageMutation::CreateScene {
                project_id: first,
                scene_id: SceneId::new_v4(),
                parent_scene_id: None,
                placement_id: None,
            })
            .unwrap();
        queue
            .submit(ProjectStageMutation::CreateScene {
                project_id: second,
                scene_id: SceneId::new_v4(),
                parent_scene_id: None,
                placement_id: None,
            })
            .unwrap();
        assert_eq!(queue.pending_len(), 2);
    }
}
