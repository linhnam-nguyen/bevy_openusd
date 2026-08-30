use std::{
    io::Read,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use openusd::usd::Stage;
use project_protocol::{
    LocalSelectionToken, ProjectCommitRequest, ProjectCommitTarget, ProjectExportSceneRequest,
    ProjectWriteTarget,
};
use tempfile::tempdir;
use usd_bevy::{LiveRevision, LiveStage};
use usd_git::GitRepository;

use super::{
    ProjectApplicationService, ProjectPublicationCoordinator, ProjectRuntimeAuthority,
    ProjectRuntimeSnapshot,
};

#[derive(Default)]
struct RecordingRuntimeAuthority {
    commit_snapshot: Mutex<Option<ProjectRuntimeSnapshot>>,
    export_snapshot: Mutex<Option<ProjectRuntimeSnapshot>>,
    begin_calls: AtomicUsize,
    validate_calls: AtomicUsize,
    finish_calls: AtomicUsize,
}

impl RecordingRuntimeAuthority {
    fn set_commit_snapshot(&self, snapshot: ProjectRuntimeSnapshot) {
        *self.commit_snapshot.lock().unwrap() = Some(snapshot);
    }

    fn set_export_snapshot(&self, snapshot: ProjectRuntimeSnapshot) {
        *self.export_snapshot.lock().unwrap() = Some(snapshot);
    }
}

impl ProjectRuntimeAuthority for RecordingRuntimeAuthority {
    fn begin_commit(
        &self,
        _project_root: &std::path::Path,
        _project_id: usd_project::ProjectId,
        _target: &ProjectCommitTarget,
    ) -> Result<Option<ProjectRuntimeSnapshot>, project_protocol::ProjectWriteError> {
        self.begin_calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.commit_snapshot.lock().unwrap().clone())
    }

    fn validate_commit(
        &self,
        _project_root: &std::path::Path,
        _project_id: usd_project::ProjectId,
        _lease_id: &str,
        _live_revision: LiveRevision,
    ) -> Result<(), project_protocol::ProjectWriteError> {
        self.validate_calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn finish_commit(
        &self,
        _project_root: &std::path::Path,
        _project_id: usd_project::ProjectId,
        _lease_id: &str,
        revision: &str,
        _live_revision: LiveRevision,
    ) -> Result<(), project_protocol::ProjectWriteError> {
        assert_eq!(revision.len(), 40);
        self.finish_calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn abort_commit(
        &self,
        _project_root: &std::path::Path,
        _project_id: usd_project::ProjectId,
        _lease_id: &str,
    ) {
    }

    fn snapshot_for_export(
        &self,
        _project_root: &std::path::Path,
        _project_id: usd_project::ProjectId,
        _scene_id: usd_project::SceneId,
    ) -> Result<Option<ProjectRuntimeSnapshot>, project_protocol::ProjectWriteError> {
        Ok(self.export_snapshot.lock().unwrap().clone())
    }
}

fn live_snapshot(
    project_root: &std::path::Path,
    scene_id: usd_project::SceneId,
    marker: &str,
) -> ProjectRuntimeSnapshot {
    let scene_path = crate::project::scene::authoring::scene_path(project_root, scene_id);
    let live = LiveStage::new(Stage::open(scene_path.to_string_lossy().as_ref()).unwrap());
    usd_bevy::authoring::define_prim(&live.stage, &format!("/SceneRoot/{marker}"), "Xform")
        .unwrap();
    let live_revision = live.drain_change_batch().unwrap().revision;
    ProjectRuntimeSnapshot {
        lease_id: format!("lease-{marker}"),
        session_id: live.session_id(),
        scene_id,
        live_revision,
        root_layer: live
            .stage
            .root_layer()
            .export_to_string()
            .unwrap()
            .into_bytes(),
    }
}

#[test]
fn runtime_timeout_is_busy_instead_of_disk_fallback() {
    let directory = tempdir().unwrap();
    let queue = super::ProjectRuntimeAuthorityQueue::with_timeout(Duration::from_millis(1));
    let result = queue.begin_commit(
        directory.path(),
        usd_project::ProjectId::new_v4(),
        &ProjectCommitTarget::Project,
    );
    assert!(matches!(
        result,
        Err(project_protocol::ProjectWriteError::Failed {
            code: project_protocol::ProjectWriteErrorCode::Busy
        })
    ));
    let export_result = queue.snapshot_for_export(
        directory.path(),
        usd_project::ProjectId::new_v4(),
        usd_project::SceneId::new_v4(),
    );
    assert!(matches!(
        export_result,
        Err(project_protocol::ProjectWriteError::Failed {
            code: project_protocol::ProjectWriteErrorCode::Busy
        })
    ));
}

#[test]
fn public_commit_uses_live_snapshot_and_persists_semantic_cache() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    std::fs::create_dir(&parent).unwrap();
    let authority = Arc::new(RecordingRuntimeAuthority::default());
    let coordinator = ProjectPublicationCoordinator::with_runtime_authority(authority.clone());
    let mut service = ProjectApplicationService::open_with_publication_coordinator(
        directory.path().join("workspace.json"),
        coordinator,
    )
    .unwrap();
    let project = service.create_project(&parent, "Live Commit").unwrap();
    let scene = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Active Scene",
        )
        .unwrap();
    let project_root = parent.join("Live Commit");
    authority.set_commit_snapshot(live_snapshot(
        &project_root,
        scene.scene_id,
        "LiveOnlyPublicCommit",
    ));

    let response = service
        .commit(ProjectCommitRequest {
            project_id: project.id,
            target: ProjectCommitTarget::Project,
            message: "public live commit".to_owned(),
        })
        .unwrap();
    let repository = usd_git::Repository::open(&project_root).unwrap();
    let materialized = tempdir().unwrap();
    repository
        .materialize_revision(
            &usd_git::RevisionId::new(response.revision.id),
            materialized.path(),
        )
        .unwrap();
    let committed_scene = std::fs::read_to_string(
        materialized
            .path()
            .join(".usdhub/scenes")
            .join(format!("{}.usda", scene.scene_id)),
    )
    .unwrap();
    assert!(committed_scene.contains("LiveOnlyPublicCommit"));
    assert_eq!(authority.begin_calls.load(Ordering::Relaxed), 1);
    assert_eq!(authority.validate_calls.load(Ordering::Relaxed), 1);
    assert_eq!(authority.finish_calls.load(Ordering::Relaxed), 1);
    assert!(
        project_root
            .join(".usdhub/cache/semantic-snapshots.db")
            .is_file(),
        "public live commit should reach durable semantic persistence"
    );
}

#[test]
fn public_export_uses_the_active_live_snapshot() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    std::fs::create_dir(&parent).unwrap();
    let authority = Arc::new(RecordingRuntimeAuthority::default());
    let coordinator = ProjectPublicationCoordinator::with_runtime_authority(authority.clone());
    let mut service = ProjectApplicationService::open_with_publication_coordinator(
        directory.path().join("workspace.json"),
        coordinator,
    )
    .unwrap();
    let project = service.create_project(&parent, "Live Export").unwrap();
    let scene = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Active Scene",
        )
        .unwrap();
    let project_root = parent.join("Live Export");
    authority.set_export_snapshot(live_snapshot(
        &project_root,
        scene.scene_id,
        "LiveOnlyPublicExport",
    ));
    let destination = directory.path().join("live.usdz");

    service
        .export_scene(
            ProjectExportSceneRequest {
                project_id: project.id,
                scene_id: scene.scene_id,
                destination: LocalSelectionToken::new("destination"),
            },
            &destination,
        )
        .unwrap();

    let file = std::fs::File::open(destination).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let mut root = String::new();
    archive
        .by_name("scene.usda")
        .unwrap()
        .read_to_string(&mut root)
        .unwrap();
    assert!(root.contains("LiveOnlyPublicExport"));
}

#[test]
fn scene_commit_includes_dirty_active_ancestor_snapshot() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    std::fs::create_dir(&parent).unwrap();
    let authority = Arc::new(RecordingRuntimeAuthority::default());
    let coordinator = ProjectPublicationCoordinator::with_runtime_authority(authority.clone());
    let mut service = ProjectApplicationService::open_with_publication_coordinator(
        directory.path().join("workspace.json"),
        coordinator,
    )
    .unwrap();
    let project = service.create_project(&parent, "Closure Commit").unwrap();
    let ancestor = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Ancestor",
        )
        .unwrap();
    let child = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Scene(ancestor.scene_id),
            "Child",
        )
        .unwrap();
    let project_root = parent.join("Closure Commit");
    authority.set_commit_snapshot(live_snapshot(
        &project_root,
        ancestor.scene_id,
        "DirtyActiveAncestor",
    ));

    let response = service
        .commit(ProjectCommitRequest {
            project_id: project.id,
            target: ProjectCommitTarget::Scene(child.scene_id),
            message: "closure commit".to_owned(),
        })
        .unwrap();
    let repository = usd_git::Repository::open(&project_root).unwrap();
    let materialized = tempdir().unwrap();
    repository
        .materialize_revision(
            &usd_git::RevisionId::new(response.revision.id),
            materialized.path(),
        )
        .unwrap();
    let ancestor_layer = std::fs::read_to_string(
        materialized
            .path()
            .join(".usdhub/scenes")
            .join(format!("{}.usda", ancestor.scene_id)),
    )
    .unwrap();
    assert!(ancestor_layer.contains("DirtyActiveAncestor"));
}

#[test]
fn export_parent_includes_dirty_active_descendant_snapshot() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    std::fs::create_dir(&parent).unwrap();
    let authority = Arc::new(RecordingRuntimeAuthority::default());
    let coordinator = ProjectPublicationCoordinator::with_runtime_authority(authority.clone());
    let mut service = ProjectApplicationService::open_with_publication_coordinator(
        directory.path().join("workspace.json"),
        coordinator,
    )
    .unwrap();
    let project = service.create_project(&parent, "Closure Export").unwrap();
    let ancestor = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Ancestor",
        )
        .unwrap();
    let child = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Scene(ancestor.scene_id),
            "Child",
        )
        .unwrap();
    let project_root = parent.join("Closure Export");
    authority.set_export_snapshot(live_snapshot(
        &project_root,
        child.scene_id,
        "DirtyActiveDescendant",
    ));
    let destination = directory.path().join("closure.usdz");

    service
        .export_scene(
            ProjectExportSceneRequest {
                project_id: project.id,
                scene_id: ancestor.scene_id,
                destination: LocalSelectionToken::new("destination"),
            },
            &destination,
        )
        .unwrap();

    let file = std::fs::File::open(destination).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let mut child_layer = String::new();
    archive
        .by_name(&format!("scenes/{}.usda", child.scene_id))
        .unwrap()
        .read_to_string(&mut child_layer)
        .unwrap();
    assert!(child_layer.contains("DirtyActiveDescendant"));
}
