use std::fs;

use openusd::usd::Stage;
use project_protocol::{ProjectWriteError, ProjectWriteErrorCode, ProjectWriteTarget};
use tempfile::tempdir;

use super::ProjectApplicationService;
use crate::project::catalog::manifest_store::ManifestStore;

#[test]
fn create_scene_places_under_the_protected_project_root() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let registry_path = directory.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(registry_path).unwrap();
    let project = service.create_project(&parent, "Scene Project").unwrap();
    let project_root = parent.join("Scene Project");

    let created = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Main Scene",
        )
        .unwrap();

    let protected_root = match project.root {
        usd_project::ProjectRoot::Scene(scene_id) => scene_id,
        _ => panic!("new Project must have a protected Root Scene"),
    };
    assert!(created.placement_id.is_some());
    assert_eq!(created.project.root, project.root);
    assert_ne!(protected_root, created.scene_id);
    let manifest = ManifestStore::read_validated(&parent.join("Scene Project")).unwrap();
    assert_eq!(manifest.scenes().len(), 2);
    assert_eq!(manifest.raw().root, created.project.root);
    let created_scene = manifest.scene(created.scene_id).unwrap();
    assert!(
        project_root
            .join("scenes")
            .join(format!("{}.usda", created_scene.storage_key))
            .is_file()
    );
    assert!(!project_root.join(".usdhub/scenes").exists());
    let members = crate::project::scene::authoring::read_scene_members(
        &crate::project::scene::authoring::scene_path(&project_root, protected_root),
        protected_root,
    )
    .unwrap();
    assert!(members.iter().any(|member| {
        member.target == usd_project::SceneMemberTarget::Scene(created.scene_id)
    }));
    assert!(
        parent
            .join("Scene Project/.usdhub/cache/project-stage-mutations")
            .exists()
    );
}

#[test]
fn create_scene_adds_one_identity_preserving_child_placement() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let registry_path = directory.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(registry_path).unwrap();
    let project = service
        .create_project(&parent, "Nested Scene Project")
        .unwrap();
    let root = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Main Scene",
        )
        .unwrap();

    let child = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Scene(root.scene_id),
            "Child Scene",
        )
        .unwrap();

    let project_root = parent.join("Nested Scene Project");
    let members = crate::project::scene::authoring::read_scene_members(
        &crate::project::scene::authoring::scene_path(&project_root, root.scene_id),
        root.scene_id,
    )
    .unwrap();
    assert_eq!(child.placement_id, members.first().map(|member| member.id));
    assert!(matches!(
        members.as_slice(),
        [usd_project::SceneMember {
            target: usd_project::SceneMemberTarget::Scene(target),
            name: Some(name),
            ..
        }] if *target == child.scene_id && name == "Child Scene"
    ));
    let parent_stage = Stage::open(
        &crate::project::scene::authoring::scene_path(&project_root, root.scene_id)
            .to_string_lossy(),
    )
    .unwrap();
    let authored = parent_stage.root_layer().export_to_string().unwrap();
    assert!(authored.contains("references"));
    assert!(authored.contains(&child.scene_id.to_string()));
    assert_eq!(
        ManifestStore::read_validated(&project_root)
            .unwrap()
            .scenes()
            .len(),
        3
    );
}

#[test]
fn create_scene_rejects_model_root_and_invalid_names_without_mutation() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let registry_path = directory.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(registry_path).unwrap();
    let project = service
        .create_project(&parent, "Guarded Scene Project")
        .unwrap();
    let project_root = parent.join("Guarded Scene Project");

    let before = ManifestStore::read_validated(&project_root)
        .unwrap()
        .raw()
        .clone();
    assert_eq!(
        service.create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "../escape",
        ),
        Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::InvalidSceneName
        })
    );
    assert_eq!(
        ManifestStore::read_validated(&project_root).unwrap().raw(),
        &before
    );

    let model_id = usd_project::ModelId::new_v4();
    let mut model_root = before;
    model_root.models.push(usd_project::ModelManifestEntry {
        id: model_id,
        source_kind: usd_project::ModelSourceKind::Usd,
        storage_key: usd_project::StorageKey::new("model").unwrap(),
        display_name: "Model".to_owned(),
    });
    model_root.root = usd_project::ProjectRoot::Model(model_id);
    ManifestStore::write_manifest_atomic(&project_root, &model_root).unwrap();
    assert_eq!(
        service.create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Blocked Scene",
        ),
        Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::InvalidRootForComposition
        })
    );
}
