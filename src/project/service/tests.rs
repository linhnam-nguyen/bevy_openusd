use std::{
    fs,
    sync::{Arc, Barrier, mpsc},
};

use project_protocol::{
    ProjectListItem, ProjectReadCommand, ProjectReadError, ProjectReadRequest, ProjectReadResponse,
    ProjectStageTarget,
};
use tempfile::tempdir;
use usd_project::{
    ModelId, ModelManifestEntry, ModelSourceKind, ProjectContentNode, ProjectId, ProjectManifestV1,
    ProjectRoot, SceneId, SceneManifestEntry, SceneMember, SceneMemberId, SceneMemberTarget,
    StorageKey,
};

use super::{
    ProjectApplicationService, ProjectImportProgressStore, ProjectPublicationCoordinator,
    ProjectStageMutationQueue,
};
use crate::project::catalog::{
    manifest_store::ManifestStore, workspace_registry::WorkspaceRegistry,
};

#[test]
fn unknown_project_id_returns_typed_not_found_without_a_path() {
    let directory = tempdir().unwrap();
    let registry = WorkspaceRegistry::load(directory.path().join("workspace.json")).unwrap();
    let service = ProjectApplicationService {
        registry,
        publication_coordinator: ProjectPublicationCoordinator::default(),
        stage_mutations: ProjectStageMutationQueue::default(),
        progress: ProjectImportProgressStore::default(),
        cache_warm: crate::project::cache_warmer::ProjectCacheWarmQueue::default(),
    };
    let project_id = ProjectId::new_v4();

    let reply = service.execute(ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(
        project_id,
    )));

    assert_eq!(reply.result, Err(ProjectReadError::NotFound { project_id }));
    assert!(!format!("{reply:?}").contains(directory.path().to_string_lossy().as_ref()));
}

#[test]
fn list_projects_returns_owned_summaries_from_the_registry() {
    let directory = tempdir().unwrap();
    let registry_path = directory.path().join("workspace.json");
    let project_id = ProjectId::new_v4();
    let repository = directory.path().join("repository");
    let manifest = ProjectManifestV1::new(
        project_id,
        "Project",
        ProjectRoot::Empty,
        Vec::new(),
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(&repository, &manifest).unwrap();
    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry.register(project_id, &repository, None).unwrap();
    let service = ProjectApplicationService {
        registry,
        publication_coordinator: ProjectPublicationCoordinator::default(),
        stage_mutations: ProjectStageMutationQueue::default(),
        progress: ProjectImportProgressStore::default(),
        cache_warm: crate::project::cache_warmer::ProjectCacheWarmQueue::default(),
    };

    let reply = service.execute(ProjectReadCommand::new(ProjectReadRequest::ListProjects));
    let ProjectReadResponse::Projects(items) = reply.result.unwrap() else {
        panic!("ListProjects must return catalogue items");
    };
    assert!(matches!(items.as_slice(), [ProjectListItem::Available(_)]));
}

#[test]
fn unavailable_project_keeps_registry_identity_and_reports_missing_repository() {
    let directory = tempdir().unwrap();
    let registry_path = directory.path().join("workspace.json");
    let project_id = ProjectId::new_v4();
    let missing_repository = directory.path().join("missing-repository");
    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry
        .register(project_id, &missing_repository, None)
        .unwrap();
    let service = ProjectApplicationService {
        registry,
        publication_coordinator: ProjectPublicationCoordinator::default(),
        stage_mutations: ProjectStageMutationQueue::default(),
        progress: ProjectImportProgressStore::default(),
        cache_warm: crate::project::cache_warmer::ProjectCacheWarmQueue::default(),
    };

    let list = service.execute(ProjectReadCommand::new(ProjectReadRequest::ListProjects));
    assert_eq!(
        list.result,
        Ok(ProjectReadResponse::Projects(vec![
            ProjectListItem::Unavailable {
                project_id,
                code: project_protocol::ProjectReadErrorCode::RepositoryMissing,
            }
        ]))
    );

    let tree = service.execute(ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(
        project_id,
    )));
    assert_eq!(
        tree.result,
        Err(ProjectReadError::Unavailable {
            project_id,
            code: project_protocol::ProjectReadErrorCode::RepositoryMissing,
        })
    );
    assert!(service.registry.get(project_id).is_some());
}

#[test]
fn stage_activation_resolves_the_registered_project_root_to_a_canonical_stage() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let registry_path = directory.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(registry_path).unwrap();
    let project = service
        .create_project(&parent, "Activation Project")
        .unwrap();
    let created = service
        .create_scene(
            project.id,
            project_protocol::ProjectWriteTarget::Project(project.id),
            "Main Scene",
        )
        .unwrap();

    let target = service
        .resolve_stage_activation(
            project.id,
            ProjectStageTarget::ProjectRoot(created.project.root.clone()),
        )
        .unwrap()
        .expect("a non-empty Project root must resolve to a stage");

    assert_eq!(target.project_id, project.id);
    assert_eq!(
        target.target,
        ProjectStageTarget::ProjectRoot(created.project.root)
    );
    assert_eq!(target.path, fs::canonicalize(target.path.clone()).unwrap());
    assert!(target.path.is_file());
}

#[test]
fn stage_activation_resolves_project_and_standalone_scene_targets_without_flattening() {
    let directory = tempdir().unwrap();
    let projects = directory.path().join("projects");
    fs::create_dir(&projects).unwrap();
    let registry_path = directory.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(registry_path).unwrap();
    let project = service
        .create_project(&projects, "Activation Targets")
        .unwrap();
    let root = service
        .create_scene(
            project.id,
            project_protocol::ProjectWriteTarget::Project(project.id),
            "Root Scene",
        )
        .unwrap();
    let nested = service
        .create_scene(
            project.id,
            project_protocol::ProjectWriteTarget::Scene(root.scene_id),
            "Nested Scene",
        )
        .unwrap();

    let project_stage = service
        .resolve_stage_activation(
            project.id,
            ProjectStageTarget::ProjectRoot(project.root.clone()),
        )
        .unwrap()
        .unwrap();
    let standalone_root = service
        .resolve_stage_activation(project.id, ProjectStageTarget::Scene(root.scene_id))
        .unwrap()
        .unwrap();
    let standalone_nested = service
        .resolve_stage_activation(project.id, ProjectStageTarget::Scene(nested.scene_id))
        .unwrap()
        .unwrap();

    assert_ne!(project_stage.path, standalone_root.path);
    assert_eq!(
        project_stage.target,
        ProjectStageTarget::ProjectRoot(project.root.clone())
    );
    assert_eq!(
        standalone_root.target,
        ProjectStageTarget::Scene(root.scene_id)
    );
    assert_eq!(
        standalone_nested.target,
        ProjectStageTarget::Scene(nested.scene_id)
    );
    assert_ne!(standalone_root.path, standalone_nested.path);

    assert_eq!(
        service
            .resolve_stage_activation(
                project.id,
                ProjectStageTarget::ProjectRoot(ProjectRoot::Scene(nested.scene_id)),
            )
            .unwrap_err(),
        ProjectReadError::Unavailable {
            project_id: project.id,
            code: project_protocol::ProjectReadErrorCode::InvalidProjectData,
        }
    );
    assert_eq!(
        service
            .resolve_stage_activation(project.id, ProjectStageTarget::Scene(SceneId::new_v4()))
            .unwrap_err(),
        ProjectReadError::Unavailable {
            project_id: project.id,
            code: project_protocol::ProjectReadErrorCode::InvalidProjectData,
        }
    );
}

#[test]
fn new_project_root_has_a_stage_activation_target() {
    let directory = tempdir().unwrap();
    let projects = directory.path().join("projects");
    fs::create_dir(&projects).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let project = service
        .create_project(&projects, "Empty Activation")
        .unwrap();

    let target = service
        .resolve_stage_activation(
            project.id,
            ProjectStageTarget::ProjectRoot(project.root.clone()),
        )
        .unwrap()
        .expect("new Project must resolve its protected Root Scene");
    assert!(target.path.is_file());
}

#[test]
fn tree_projection_keeps_authored_model_placement_identity() {
    let directory = tempdir().unwrap();
    let registry_path = directory.path().join("workspace.json");
    let project_id = ProjectId::new_v4();
    let scene_id = SceneId::new_v4();
    let model_id = ModelId::new_v4();
    let member_id = SceneMemberId::new_v4();
    let repository = directory.path().join("repository");
    let manifest = ProjectManifestV1::new(
        project_id,
        "Project",
        ProjectRoot::Scene(scene_id),
        vec![SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("scene").unwrap(),
        }],
        vec![ModelManifestEntry {
            id: model_id,
            source_kind: ModelSourceKind::Usd,
            storage_key: StorageKey::new("model").unwrap(),
        }],
    );
    ManifestStore::write_manifest_atomic(&repository, &manifest).unwrap();
    crate::project::scene::authoring::author_scene_atomic_with_members(
        &repository,
        scene_id,
        &[SceneMember {
            id: member_id,
            target: SceneMemberTarget::Model(model_id),
            name: Some("Placed model".to_owned()),
            transform: Default::default(),
        }],
    )
    .unwrap();
    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry.register(project_id, &repository, None).unwrap();
    let service = ProjectApplicationService {
        registry,
        publication_coordinator: ProjectPublicationCoordinator::default(),
        stage_mutations: ProjectStageMutationQueue::default(),
        progress: ProjectImportProgressStore::default(),
        cache_warm: crate::project::cache_warmer::ProjectCacheWarmQueue::default(),
    };

    let reply = service.execute(ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(
        project_id,
    )));
    let ProjectReadResponse::ProjectTree { nodes, counts, .. } = reply.result.unwrap() else {
        panic!("GetProjectTree must return ProjectTree");
    };

    assert_eq!(counts.scenes, 1);
    assert_eq!(counts.models, 1);
    assert_eq!(counts.scene_placements, 0);
    assert_eq!(counts.model_placements, 1);

    assert!(nodes.iter().any(|node| {
        matches!(
            node,
            ProjectContentNode::ModelPlacement {
                member_id: id,
                target,
                parent_scene_id,
                ..
            } if *id == member_id && *target == model_id && *parent_scene_id == scene_id
        )
    }));
}

#[test]
fn publication_admission_is_shared_per_project_and_not_globally_serialized() {
    let coordinator = ProjectPublicationCoordinator::default();
    let project_id = ProjectId::new_v4();
    let other_project_id = ProjectId::new_v4();
    let publisher = coordinator.publisher(project_id);
    let same_project_publisher = coordinator.publisher(project_id);
    let other_project_publisher = coordinator.publisher(other_project_id);

    assert!(Arc::ptr_eq(&publisher, &same_project_publisher));
    assert!(!Arc::ptr_eq(&publisher, &other_project_publisher));

    let guard = publisher.lock().unwrap();
    let (sender, receiver) = mpsc::channel();
    let barrier = Arc::new(Barrier::new(2));
    let worker_barrier = Arc::clone(&barrier);
    let worker_publisher = Arc::clone(&same_project_publisher);
    let worker = std::thread::spawn(move || {
        worker_barrier.wait();
        let _guard = worker_publisher.lock().unwrap();
        sender.send(()).unwrap();
    });

    barrier.wait();
    assert!(receiver.try_recv().is_err());
    drop(guard);
    receiver
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("same Project publisher must be admitted after release");
    worker.join().unwrap();
}

#[path = "stage_activation_tests.rs"]
mod stage_activation;
