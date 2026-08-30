//! Application-to-LiveStage authority boundary for Project mutations.
//!
//! The native Project host and the render server are separate processes. The
//! host therefore publishes typed, durable mutation records into the private
//! Project cache directory; the active-stage owner consumes those records on
//! the LiveStage thread. OpenUSD and Bevy state never cross this boundary.

use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use project_protocol::{ProjectWriteError, ProjectWriteTarget};
use serde::{Deserialize, Serialize};
use usd_project::{ModelId, ProjectId, SceneId, SceneMember, SceneMemberId};

#[path = "stage_mutation_outbox.rs"]
mod outbox;
#[path = "stage_rename.rs"]
mod stage_rename;
use outbox::{
    busy_error, filesystem_error, outbox_path, read_pending, submit_batch_locked_with_failure,
};

/// A typed Project mutation waiting for the active-stage owner.
///
/// This DTO contains only stable Project identities. The OpenUSD Stage and
/// Bevy entities remain owned by the active-stage thread.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum ProjectStageMutation {
    CreateScene {
        project_id: ProjectId,
        scene_id: SceneId,
        parent_scene_id: Option<SceneId>,
        placement: Option<SceneMember>,
    },
    AdoptScene {
        project_id: ProjectId,
        scene_id: SceneId,
        parent_scene_id: Option<SceneId>,
        placement: Option<SceneMember>,
    },
    PublishModel {
        project_id: ProjectId,
        model_id: ModelId,
        parent_scene_id: Option<SceneId>,
        placement: Option<SceneMember>,
    },
    RemoveScenePlacement {
        project_id: ProjectId,
        parent_scene_id: SceneId,
        placement_id: SceneMemberId,
    },
    DeleteScene {
        project_id: ProjectId,
        scene_id: SceneId,
    },
    DeleteModel {
        project_id: ProjectId,
        model_id: ModelId,
    },
    Rename {
        project_id: ProjectId,
        target: ProjectWriteTarget,
        name: String,
    },
}

const STAGE_MUTATION_CAPACITY: usize = 128;
const PROJECT_METADATA_DIRECTORY: &str = ".usdhub";
const CACHE_DIRECTORY: &str = "cache";
const OUTBOX_DIRECTORY: &str = "project-stage-mutations";

/// Durable host-to-render-server Project stage handoff.
///
/// The mutex only protects concurrent access by one host process. The file is
/// the process boundary and is read by the active-stage owner in the viewer.
#[derive(Clone, Default)]
pub struct ProjectStageMutationQueue {
    file_lock: Arc<Mutex<()>>,
    #[cfg(test)]
    test_fail_before_index: Arc<Mutex<Option<usize>>>,
}

impl ProjectStageMutationQueue {
    /// Check capacity before a canonical publication starts. This prevents a
    /// successful disk mutation from being reported as failed because its
    /// stage handoff was discovered to be full afterward.
    pub fn ensure_capacity(&self, project_root: &Path) -> Result<(), ProjectWriteError> {
        self.ensure_capacity_for(project_root, 1)
    }

    /// Check capacity for a bounded batch before canonical files are changed.
    pub fn ensure_capacity_for(
        &self,
        project_root: &Path,
        additional: usize,
    ) -> Result<(), ProjectWriteError> {
        let _guard = self
            .file_lock
            .lock()
            .expect("Project stage mutation queue is not poisoned");
        let pending = read_pending(&outbox_path(project_root))?;
        if pending.len().saturating_add(additional) > STAGE_MUTATION_CAPACITY {
            return Err(busy_error());
        }
        Ok(())
    }

    /// Publish one typed mutation into the private Project cache outbox.
    pub fn submit_for_project(
        &self,
        project_root: &Path,
        mutation: ProjectStageMutation,
    ) -> Result<(), ProjectWriteError> {
        let _guard = self
            .file_lock
            .lock()
            .expect("Project stage mutation queue is not poisoned");
        submit_batch_locked_with_failure(
            project_root,
            std::slice::from_ref(&mutation),
            outbox::take_failure(self),
        )
    }

    /// Publish a typed mutation batch atomically into the private Project
    /// stage outbox. Existing records remain untouched if any record in the
    /// batch cannot be prepared or published.
    pub fn submit_batch_for_project(
        &self,
        project_root: &Path,
        mutations: &[ProjectStageMutation],
    ) -> Result<(), ProjectWriteError> {
        let _guard = self
            .file_lock
            .lock()
            .expect("Project stage mutation queue is not poisoned");
        submit_batch_locked_with_failure(project_root, mutations, outbox::take_failure(self))
    }

    /// Consume mutations for the active Scene on the actual LiveStage owner
    /// thread. Failed records and records for another Project or Scene remain
    /// in the outbox for a later retry.
    pub fn apply_for_active_scene(
        &self,
        live: &usd_bevy::LiveStage,
        project_root: &Path,
        active_project_id: ProjectId,
        active_scene_id: Option<SceneId>,
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
            if mutation.project_id() != active_project_id
                || !mutation.can_be_consumed_for_active_scene(active_scene_id)
            {
                continue;
            }
            let result = match mutation {
                ProjectStageMutation::DeleteScene { scene_id, .. }
                    if active_scene_id != Some(scene_id) =>
                {
                    Ok(())
                }
                mutation => apply_mutation(live, project_root, &mutation),
            };
            match result {
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

    #[cfg(test)]
    pub(crate) fn fail_before_batch_index(&self, index: usize) {
        *self
            .test_fail_before_index
            .lock()
            .expect("Project stage mutation queue test hook is not poisoned") = Some(index);
    }
}

impl ProjectStageMutation {
    fn project_id(&self) -> ProjectId {
        match self {
            Self::CreateScene { project_id, .. }
            | Self::AdoptScene { project_id, .. }
            | Self::PublishModel { project_id, .. }
            | Self::RemoveScenePlacement { project_id, .. }
            | Self::DeleteScene { project_id, .. }
            | Self::DeleteModel { project_id, .. }
            | Self::Rename { project_id, .. } => *project_id,
        }
    }

    fn parent_scene_id(&self) -> Option<SceneId> {
        match self {
            Self::CreateScene {
                parent_scene_id, ..
            }
            | Self::AdoptScene {
                parent_scene_id, ..
            }
            | Self::PublishModel {
                parent_scene_id, ..
            } => *parent_scene_id,
            Self::RemoveScenePlacement {
                parent_scene_id, ..
            } => Some(*parent_scene_id),
            Self::DeleteScene { scene_id, .. } => Some(*scene_id),
            Self::DeleteModel { .. } | Self::Rename { .. } => None,
        }
    }

    fn can_be_consumed_for_active_scene(&self, active_scene_id: Option<SceneId>) -> bool {
        match self {
            Self::DeleteScene { .. } | Self::DeleteModel { .. } | Self::Rename { .. } => true,
            _ => self.parent_scene_id() == active_scene_id,
        }
    }
}

fn apply_mutation(
    live: &usd_bevy::LiveStage,
    project_root: &Path,
    mutation: &ProjectStageMutation,
) -> Result<(), ProjectWriteError> {
    if let ProjectStageMutation::DeleteScene { .. } = mutation {
        usd_bevy::authoring::remove_prim(&live.stage, "/SceneRoot")
            .map_err(|_| filesystem_error())?;
        return Ok(());
    }
    if let ProjectStageMutation::DeleteModel { .. } = mutation {
        usd_bevy::authoring::remove_prim(&live.stage, "/ModelRoot")
            .map_err(|_| filesystem_error())?;
        return Ok(());
    }
    if let ProjectStageMutation::RemoveScenePlacement { placement_id, .. } = mutation {
        usd_bevy::authoring::remove_prim(
            &live.stage,
            crate::project::scene::authoring::scene_member_path(*placement_id).as_str(),
        )
        .map_err(|_| filesystem_error())?;
        return Ok(());
    }
    if let ProjectStageMutation::Rename { target, name, .. } = mutation {
        stage_rename::apply_rename_to_live_stage(live, target, name)?;
        return Ok(());
    }
    let placement = match mutation {
        ProjectStageMutation::CreateScene { placement, .. }
        | ProjectStageMutation::AdoptScene { placement, .. }
        | ProjectStageMutation::PublishModel { placement, .. } => placement,
        ProjectStageMutation::RemoveScenePlacement { .. }
        | ProjectStageMutation::DeleteScene { .. }
        | ProjectStageMutation::DeleteModel { .. }
        | ProjectStageMutation::Rename { .. } => unreachable!("handled above"),
    };
    let Some(placement) = placement else {
        // Empty -> root transitions are completed by normal root-stage
        // activation. There is no SceneMember to patch into the new root.
        return Ok(());
    };
    crate::project::scene::authoring::author_scene_member(&live.stage, project_root, placement)
        .map_err(|_| filesystem_error())
}

#[cfg(test)]
#[path = "stage_mutation_tests.rs"]
mod tests;
