//! Application-to-LiveStage authority boundary for Project mutations.
//!
//! The native Project host and the render server are separate processes. The
//! host therefore publishes typed, durable mutation records into the private
//! Project runtime directory; the active-stage owner consumes those records on
//! the LiveStage thread. OpenUSD and Bevy state never cross this boundary.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use openusd::{sdf, sdf::Value};
use project_protocol::ProjectWriteError;
use serde::{Deserialize, Serialize};
use usd_project::{ModelId, ProjectId, SceneId, SceneMemberId};
use uuid::Uuid;

/// A typed Project mutation waiting for the active-stage owner.
///
/// This DTO contains only stable Project identities. The OpenUSD Stage and
/// Bevy entities remain owned by the active-stage thread.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
const PROJECT_METADATA_DIRECTORY: &str = ".usdhub";
const RUNTIME_DIRECTORY: &str = "runtime";
const OUTBOX_DIRECTORY: &str = "project-stage-mutations";
const REFERENCES_FIELD: &str = "references";
const SCENE_ROOT_PATH: &str = "/SceneRoot";
const MODEL_ROOT_PATH: &str = "/ModelRoot";

/// Durable host-to-render-server Project stage handoff.
///
/// The mutex only protects concurrent access by one host process. The file is
/// the process boundary and is read by the active-stage owner in the viewer.
#[derive(Clone, Default)]
pub struct ProjectStageMutationQueue {
    file_lock: Arc<Mutex<()>>,
}

impl ProjectStageMutationQueue {
    /// Check capacity before a canonical publication starts. This prevents a
    /// successful disk mutation from being reported as failed because its
    /// stage handoff was discovered to be full afterward.
    pub fn ensure_capacity(&self, project_root: &Path) -> Result<(), ProjectWriteError> {
        let _guard = self
            .file_lock
            .lock()
            .expect("Project stage mutation queue is not poisoned");
        let pending = read_pending(&outbox_path(project_root))?;
        if pending.len() >= STAGE_MUTATION_CAPACITY {
            return Err(busy_error());
        }
        Ok(())
    }

    /// Publish one typed mutation into the private Project runtime outbox.
    pub fn submit_for_project(
        &self,
        project_root: &Path,
        mutation: ProjectStageMutation,
    ) -> Result<(), ProjectWriteError> {
        let _guard = self
            .file_lock
            .lock()
            .expect("Project stage mutation queue is not poisoned");
        let path = outbox_path(project_root);
        let pending = read_pending(&path)?;
        if pending.len() >= STAGE_MUTATION_CAPACITY {
            return Err(busy_error());
        }
        fs::create_dir_all(&path).map_err(|_| filesystem_error())?;
        let id = Uuid::new_v4();
        let temporary = path.join(format!(".{id}.tmp"));
        let final_path = path.join(format!("{id}.json"));
        let encoded = serde_json::to_vec(&mutation).map_err(|_| filesystem_error())?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| filesystem_error())?;
        file.write_all(&encoded).map_err(|_| filesystem_error())?;
        file.sync_all().map_err(|_| filesystem_error())?;
        fs::rename(&temporary, final_path).map_err(|_| filesystem_error())?;
        Ok(())
    }

    /// Consume mutations for the active Project on the actual LiveStage owner
    /// thread. Failed records and records for another Project remain in the
    /// outbox for a later retry.
    pub fn apply_for_active_project(
        &self,
        live: &usd_bevy::LiveStage,
        project_root: &Path,
        active_project_id: ProjectId,
    ) -> Result<usize, ProjectWriteError> {
        let _guard = self
            .file_lock
            .lock()
            .expect("Project stage mutation queue is not poisoned");
        let path = outbox_path(project_root);
        let pending = read_pending(&path)?;
        if pending.is_empty() {
            return Ok(0);
        }

        let mut applied = 0;
        let mut first_error = None;
        for (mutation_path, mutation) in pending {
            if mutation.project_id() != active_project_id {
                continue;
            }
            match apply_mutation(live, project_root, &mutation) {
                Ok(()) => match fs::remove_file(&mutation_path) {
                    Ok(()) => applied += 1,
                    Err(_) => {
                        if first_error.is_none() {
                            first_error = Some(filesystem_error());
                        }
                    }
                },
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(applied)
    }

    #[cfg(test)]
    fn pending_len_for_project(&self, project_root: &Path) -> usize {
        read_pending(&outbox_path(project_root)).unwrap().len()
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
    project_root: &Path,
    mutation: &ProjectStageMutation,
) -> Result<(), ProjectWriteError> {
    let (target_path, asset_path, referenced_prim) = match mutation {
        ProjectStageMutation::CreateScene {
            scene_id,
            placement_id,
            ..
        }
        | ProjectStageMutation::AdoptScene {
            scene_id,
            placement_id,
            ..
        } => (
            placement_path(*placement_id, SCENE_ROOT_PATH),
            crate::project::scene::authoring::scene_path(project_root, *scene_id),
            SCENE_ROOT_PATH,
        ),
        ProjectStageMutation::PublishModel {
            model_id,
            placement_id,
            ..
        } => (
            placement_path(*placement_id, MODEL_ROOT_PATH),
            crate::project::model_wrapper::model_wrapper_path(project_root, *model_id),
            MODEL_ROOT_PATH,
        ),
    };

    let asset_path = asset_path.to_str().ok_or_else(filesystem_error)?.to_owned();
    let reference = sdf::Reference {
        asset_path,
        prim_path: sdf::path(referenced_prim).map_err(|_| filesystem_error())?,
        ..Default::default()
    };
    live.stage
        .define_prim(target_path.as_str())
        .map_err(|_| filesystem_error())?
        .set_type_name("Xform")
        .map_err(|_| filesystem_error())?
        .set_metadata(
            REFERENCES_FIELD,
            Value::ReferenceListOp(sdf::ReferenceListOp::prepended([reference])),
        )
        .map_err(|_| filesystem_error())?;
    Ok(())
}

fn placement_path(placement_id: Option<SceneMemberId>, root_path: &str) -> String {
    placement_id.map_or_else(
        || root_path.to_owned(),
        crate::project::scene::authoring::scene_member_path,
    )
}

fn outbox_path(project_root: &Path) -> PathBuf {
    project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join(RUNTIME_DIRECTORY)
        .join(OUTBOX_DIRECTORY)
}

fn read_pending(path: &Path) -> Result<Vec<(PathBuf, ProjectStageMutation)>, ProjectWriteError> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(filesystem_error()),
    };
    let mut paths = entries
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|_| filesystem_error())
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "json")
    });
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path).map_err(|_| filesystem_error())?;
            let mutation = serde_json::from_slice(&bytes).map_err(|_| filesystem_error())?;
            Ok((path, mutation))
        })
        .collect()
}

fn busy_error() -> ProjectWriteError {
    ProjectWriteError::Failed {
        code: project_protocol::ProjectWriteErrorCode::Busy,
    }
}

fn filesystem_error() -> ProjectWriteError {
    ProjectWriteError::Failed {
        code: project_protocol::ProjectWriteErrorCode::FilesystemFailure,
    }
}

#[cfg(test)]
mod tests {
    use openusd::usd::Stage;

    use super::*;

    fn stage() -> usd_bevy::LiveStage {
        usd_bevy::LiveStage::new(
            Stage::builder()
                .in_memory("project-active-stage.usda")
                .unwrap(),
        )
    }

    #[test]
    fn canonical_project_mutations_reach_live_stage_as_real_references() {
        let directory = tempfile::tempdir().unwrap();
        let project_root = directory.path().join("Project");
        fs::create_dir_all(&project_root).unwrap();
        let project_id = ProjectId::new_v4();
        let scene_id = SceneId::new_v4();
        let placement_id = SceneMemberId::new_v4();
        let model_id = ModelId::new_v4();
        let queue = ProjectStageMutationQueue::default();

        queue
            .submit_for_project(
                &project_root,
                ProjectStageMutation::AdoptScene {
                    project_id,
                    scene_id,
                    parent_scene_id: Some(SceneId::new_v4()),
                    placement_id: Some(placement_id),
                },
            )
            .unwrap();
        queue
            .submit_for_project(
                &project_root,
                ProjectStageMutation::PublishModel {
                    project_id,
                    model_id,
                    parent_scene_id: Some(SceneId::new_v4()),
                    placement_id: Some(SceneMemberId::new_v4()),
                },
            )
            .unwrap();

        let live = stage();
        assert_eq!(
            queue
                .apply_for_active_project(&live, &project_root, project_id)
                .unwrap(),
            2
        );
        let batch = live
            .drain_change_batch()
            .expect("real Project change batch");
        assert!(batch.has_resync());
        let exported = live.stage.root_layer().export_to_string().unwrap();
        assert!(exported.contains("references"));
        assert!(exported.contains(&scene_id.to_string()));
        assert!(exported.contains(&model_id.to_string()));
        assert!(exported.contains(&placement_id.to_string().replace('-', "")));
        assert!(!exported.contains("/__usdhub/project_"));
        assert_eq!(queue.pending_len_for_project(&project_root), 0);
    }

    #[test]
    fn inactive_project_outbox_remains_isolated() {
        let directory = tempfile::tempdir().unwrap();
        let first_root = directory.path().join("first");
        let second_root = directory.path().join("second");
        fs::create_dir_all(&first_root).unwrap();
        fs::create_dir_all(&second_root).unwrap();
        let queue = ProjectStageMutationQueue::default();
        let first = ProjectId::new_v4();
        let second = ProjectId::new_v4();
        for (root, project_id) in [(&first_root, first), (&second_root, second)] {
            queue
                .submit_for_project(
                    root,
                    ProjectStageMutation::CreateScene {
                        project_id,
                        scene_id: SceneId::new_v4(),
                        parent_scene_id: None,
                        placement_id: None,
                    },
                )
                .unwrap();
        }

        let live = stage();
        queue
            .apply_for_active_project(&live, &first_root, first)
            .unwrap();
        assert_eq!(queue.pending_len_for_project(&first_root), 0);
        assert_eq!(queue.pending_len_for_project(&second_root), 1);
    }
}
