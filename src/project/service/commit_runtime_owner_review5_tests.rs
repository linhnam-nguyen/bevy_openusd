use std::{
    fs,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use project_protocol::{
    ProjectCommitRequest, ProjectCommitTarget, ProjectWriteError, ProjectWriteErrorCode,
    ProjectWriteTarget,
};
use usd_bevy::LiveRevision;
use usd_project::{ProjectId, SceneId};

use super::super::{
    ProjectApplicationService, ProjectPublicationCoordinator, ProjectRuntimeAuthority,
    ProjectRuntimeSnapshot,
};

struct FailingFinishAuthority {
    snapshot: Mutex<Option<ProjectRuntimeSnapshot>>,
    abort_count: AtomicUsize,
}

impl FailingFinishAuthority {
    fn set_snapshot(&self, snapshot: ProjectRuntimeSnapshot) {
        *self.snapshot.lock().unwrap() = Some(snapshot);
    }
}

impl ProjectRuntimeAuthority for FailingFinishAuthority {
    fn begin_commit(
        &self,
        _: &Path,
        _: ProjectId,
        _: &ProjectCommitTarget,
    ) -> Result<Option<ProjectRuntimeSnapshot>, ProjectWriteError> {
        Ok(self.snapshot.lock().unwrap().clone())
    }
    fn finish_commit(
        &self,
        _: &Path,
        _: ProjectId,
        _: &str,
        _: &str,
        _: LiveRevision,
    ) -> Result<(), ProjectWriteError> {
        Err(ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::Busy,
        })
    }
    fn validate_commit(
        &self,
        _: &Path,
        _: ProjectId,
        _: &str,
        _: LiveRevision,
    ) -> Result<(), ProjectWriteError> {
        Ok(())
    }
    fn abort_commit(&self, _: &Path, _: ProjectId, _: &str) {
        self.abort_count.fetch_add(1, Ordering::Relaxed);
    }
    fn snapshot_for_export(
        &self,
        _: &Path,
        _: ProjectId,
        _: SceneId,
    ) -> Result<Option<ProjectRuntimeSnapshot>, ProjectWriteError> {
        Ok(None)
    }
}

#[test]
fn failed_finish_on_commit_path_aborts_runtime_lease() {
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let authority = Arc::new(FailingFinishAuthority {
        snapshot: Mutex::new(None),
        abort_count: AtomicUsize::new(0),
    });
    let coordinator = ProjectPublicationCoordinator::with_runtime_authority(authority.clone());
    let mut service = ProjectApplicationService::open_with_publication_coordinator(
        directory.path().join("workspace.json"),
        coordinator,
    )
    .unwrap();
    let project = service.create_project(&parent, "Finish Failure").unwrap();
    let scene = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Active Scene",
        )
        .unwrap();
    let project_root = parent.join("Finish Failure");
    let scene_path = crate::project::scene::authoring::scene_path(&project_root, scene.scene_id);
    authority.set_snapshot(ProjectRuntimeSnapshot {
        lease_id: "lease-finish-failure".to_owned(),
        session_id: 1,
        scene_id: scene.scene_id,
        live_revision: LiveRevision(0),
        root_layer: fs::read(scene_path).unwrap(),
    });

    let response = service
        .commit(ProjectCommitRequest {
            project_id: project.id,
            target: ProjectCommitTarget::Project,
            message: "finish failure regression".to_owned(),
        })
        .unwrap();

    assert_eq!(response.revision.id.len(), 40);
    assert_eq!(authority.abort_count.load(Ordering::Relaxed), 1);
}
