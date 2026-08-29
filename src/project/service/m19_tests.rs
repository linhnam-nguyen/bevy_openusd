use std::fs;

use project_protocol::{
    ProjectListItem, ProjectReadCommand, ProjectReadRequest, ProjectReadResponse,
    ProjectStageTarget, ProjectWriteTarget,
};
use tempfile::tempdir;
use usd_git::GitRepository;
use usd_project::{ProjectContentNode, ProjectRoot};

use super::{ProjectApplicationService, ProjectModelPreparationQueue};

#[test]
fn phase2_freeze_matrix_covers_create_import_composition_and_recovery() {
    let directory = tempdir().unwrap();
    let projects = directory.path().join("projects");
    fs::create_dir(&projects).unwrap();
    let mut service = ProjectApplicationService::open(directory.path().join("workspace.json"))
        .expect("open Project service");

    let created = service
        .create_project(&projects, "End-to-end Project")
        .expect("create Project");
    let project_root = projects.join("End-to-end Project");
    let repository = usd_git::Repository::open(&project_root).unwrap();
    assert_eq!(
        repository.current_branch().unwrap().as_deref(),
        Some("main")
    );
    assert!(
        repository.head().unwrap().is_none(),
        "created Project stays unborn"
    );
    assert!(project_root.join(".usdhub/cache").is_dir());
    assert!(project_root.join(".usdhub/recovery").is_dir());

    let root = service
        .create_scene(
            created.id,
            ProjectWriteTarget::Project(created.id),
            "Main Scene",
        )
        .expect("create root Scene");
    let child = service
        .create_scene(
            created.id,
            ProjectWriteTarget::Scene(root.scene_id),
            "Child Scene",
        )
        .expect("create nested Scene");
    assert!(child.placement_id.is_some());
    assert_eq!(root.project.root, created.root);
    assert_ne!(root.project.root, ProjectRoot::Scene(root.scene_id));

    let model_source = directory.path().join("assembly.usda");
    fs::write(
        &model_source,
        "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (kind = \"component\") {}\n",
    )
    .unwrap();
    let preparation = ProjectModelPreparationQueue::default();
    let prepared = preparation.prepare("m19-model".to_owned(), 1, model_source.clone());
    assert!(prepared.inspection.is_ok());
    let model = service
        .publish_model(
            &preparation,
            created.id,
            ProjectWriteTarget::Scene(child.scene_id),
            &model_source,
            "m19-model".to_owned(),
            1,
            project_protocol::PlacementSpec::Default,
        )
        .expect("publish Model into nested Scene");
    assert!(model.placement_id.is_some());

    let tree = service.execute(ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(
        created.id,
    )));
    let ProjectReadResponse::ProjectTree { nodes, counts, .. } = tree.result.unwrap() else {
        panic!("Project tree read must succeed");
    };
    assert_eq!(counts.scenes, 3);
    assert_eq!(counts.models, 1);
    assert_eq!(counts.model_placements, 1);
    assert!(nodes.iter().any(|node| matches!(
        node,
        ProjectContentNode::ModelPlacement { parent_scene_id, .. }
            if *parent_scene_id == child.scene_id
    )));

    let stage_target = service
        .resolve_stage_activation(
            created.id,
            ProjectStageTarget::ProjectRoot(model.project.root),
        )
        .expect("resolve canonical active root")
        .expect("non-empty Project has a stage target");
    assert!(stage_target.path.is_file());

    // Derived cache/recovery state is disposable; the manifest and registry
    // remain the canonical Project identity and must still list the Project.
    fs::remove_dir_all(project_root.join(".usdhub/cache")).unwrap();
    fs::remove_dir_all(project_root.join(".usdhub/recovery")).unwrap();
    let list = service.execute(ProjectReadCommand::new(ProjectReadRequest::ListProjects));
    let ProjectReadResponse::Projects(items) = list.result.unwrap() else {
        panic!("Project catalogue read must succeed");
    };
    assert!(items.iter().any(|item| matches!(
        item,
        ProjectListItem::Available(summary) if summary.id == created.id
    )));
}

#[test]
fn phase2_freeze_matrix_covers_native_and_adopted_imports_and_missing_locations() {
    let directory = tempdir().unwrap();
    let mut service = ProjectApplicationService::open(directory.path().join("workspace.json"))
        .expect("open Project service");

    let adopted_root = directory.path().join("existing-git");
    usd_git::Repository::init(&adopted_root).unwrap();
    fs::write(adopted_root.join("source.usda"), b"#usda 1.0\n").unwrap();
    let inspection = service.inspect_project(&adopted_root).unwrap();
    let adopted = service
        .import_project(&adopted_root, &inspection)
        .expect("adopt generic Git repository");
    assert!(adopted_root.join(".usdhub/project.json").is_file());
    assert!(
        service
            .execute(ProjectReadCommand::new(ProjectReadRequest::ListProjects))
            .result
            .is_ok()
    );

    let parent = directory.path().join("native-parent");
    fs::create_dir(&parent).unwrap();
    let native = service.create_project(&parent, "Native").unwrap();
    let native_path = parent.join("Native");
    let native_inspection = service.inspect_project(&native_path).unwrap();
    service
        .import_project(&native_path, &native_inspection)
        .expect("re-import native Project");

    fs::rename(&native_path, &native_path.with_file_name("Native-moved")).unwrap();
    let missing = service.execute(ProjectReadCommand::new(ProjectReadRequest::ListProjects));
    let ProjectReadResponse::Projects(items) = missing.result.unwrap() else {
        panic!("Project catalogue read must succeed with a moved location");
    };
    assert!(items.iter().any(|item| matches!(
        item,
        ProjectListItem::Unavailable { project_id, .. } if *project_id == native.id
    )));
    assert!(
        service
            .execute(ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(
                adopted.id
            )))
            .result
            .is_ok()
    );
}
