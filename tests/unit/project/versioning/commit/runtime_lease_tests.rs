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

use crate::project::service::{
    ProjectApplicationService, ProjectPublicationCoordinator, ProjectRuntimeAuthority,
    ProjectRuntimeSnapshot,
};

struct FailingFinishAuthority {
    snapshot: Mutex<Option<ProjectRuntimeSnapshot>>,
    abort_count: AtomicUsize,
    fail_finish: bool,
    validations: AtomicUsize,
    concurrent_change: Option<std::path::PathBuf>,
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
        if !self.fail_finish {
            return Ok(());
        }
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
        if self.validations.fetch_add(1, Ordering::Relaxed) == 1
            && let Some(path) = &self.concurrent_change
        {
            fs::write(path, b"concurrent Scene bytes").unwrap();
        }
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
        fail_finish: true,
        validations: AtomicUsize::new(0),
        concurrent_change: None,
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
        opened_root_digest: None,
        root_layer: fs::read(scene_path).unwrap(),
    });

    let response = service
        .commit(ProjectCommitRequest {
            project_id: project.id,
            target: ProjectCommitTarget::Project,
            message: "finish failure regression".to_owned(),
        })
        .unwrap_err();

    assert!(
        matches!(response, ProjectWriteError::LiveCommitIncomplete { revision } if revision.id.len() == 40)
    );
    assert_eq!(authority.abort_count.load(Ordering::Relaxed), 1);
}

#[test]
fn live_commit_publishes_exact_scene_or_preserves_concurrent_worktree() {
    use usd_git::{GitRepository, Repository};

    for scenario in ["clean", "preexisting", "concurrent", "unborn_conflict"] {
        let dir = tempfile::tempdir().unwrap();
        let registry = dir.path().join("workspace.json");
        let mut service = ProjectApplicationService::open(&registry).unwrap();
        let project = service.create_project(dir.path(), "Live").unwrap();
        let root = dir.path().join("Live");
        let scene = service
            .create_scene(
                project.id,
                ProjectWriteTarget::Project(project.id),
                "Managed",
            )
            .unwrap();
        let baseline = (scenario != "unborn_conflict").then(|| {
            service
                .commit(ProjectCommitRequest {
                    project_id: project.id,
                    target: ProjectCommitTarget::Project,
                    message: "baseline".to_owned(),
                })
                .unwrap()
        });
        let path = crate::project::scene::authoring::scene_path(&root, scene.scene_id);
        let opened_root_digest = *blake3::hash(&fs::read(&path).unwrap()).as_bytes();
        let stage = openusd::usd::Stage::open(path.to_str().unwrap()).unwrap();
        usd_bevy::authoring::define_prim(&stage, "/SceneRoot/LiveEdit", "Xform").unwrap();
        let root_layer = stage.root_layer().export_to_string().unwrap().into_bytes();
        let authority = Arc::new(FailingFinishAuthority {
            snapshot: Mutex::new(Some(ProjectRuntimeSnapshot {
                lease_id: "lease".to_owned(),
                session_id: 1,
                scene_id: scene.scene_id,
                live_revision: LiveRevision(1),
                opened_root_digest: Some(opened_root_digest),
                root_layer: root_layer.clone(),
            })),
            abort_count: AtomicUsize::new(0),
            fail_finish: false,
            validations: AtomicUsize::new(0),
            concurrent_change: (scenario == "concurrent").then(|| path.clone()),
        });
        if matches!(scenario, "preexisting" | "unborn_conflict") {
            let mut bytes = fs::read(&path).unwrap();
            bytes.extend(b"\n# preexisting user comment\n");
            fs::write(&path, bytes).unwrap();
        }
        fs::write(root.join("unrelated.txt"), b"unrelated user work").unwrap();
        let coordinator = ProjectPublicationCoordinator::with_runtime_authority(authority.clone());
        let result =
            ProjectApplicationService::open_with_publication_coordinator(&registry, coordinator)
                .unwrap()
                .commit(ProjectCommitRequest {
                    project_id: project.id,
                    target: ProjectCommitTarget::Scene(scene.scene_id),
                    message: "live edit".to_owned(),
                });
        let repo = Repository::open(&root).unwrap();
        let head = repo.head().unwrap();
        if matches!(scenario, "preexisting" | "unborn_conflict") {
            assert!(matches!(
                result,
                Err(ProjectWriteError::Failed {
                    code: ProjectWriteErrorCode::ConcurrentChange
                })
            ));
            assert_eq!(
                head.as_ref().map(|head| head.id().to_string()),
                baseline.map(|commit| commit.revision.id),
            );
            assert!(
                fs::read_to_string(&path)
                    .unwrap()
                    .contains("# preexisting user comment")
            );
            assert_eq!(authority.abort_count.load(Ordering::Relaxed), 1);
            continue;
        }
        let head = head.unwrap();
        let committed = dir.path().join("committed");
        repo.materialize_revision(head.id(), &committed).unwrap();
        assert_eq!(
            fs::read(committed.join(path.strip_prefix(&root).unwrap())).unwrap(),
            root_layer
        );
        assert!(!committed.join("unrelated.txt").exists());
        assert_eq!(
            fs::read(root.join("unrelated.txt")).unwrap(),
            b"unrelated user work"
        );
        if scenario == "concurrent" {
            assert!(
                matches!(result, Err(ProjectWriteError::LiveCommitIncomplete { revision }) if revision.id == head.id().to_string())
            );
            assert_eq!(fs::read(&path).unwrap(), b"concurrent Scene bytes");
            assert_eq!(authority.abort_count.load(Ordering::Relaxed), 1);
        } else {
            assert_eq!(result.unwrap().revision.id, head.id().to_string());
            assert_eq!(fs::read(&path).unwrap(), root_layer);
            assert_eq!(authority.abort_count.load(Ordering::Relaxed), 0);
        }
    }
}
