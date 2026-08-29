use super::*;

#[test]
fn stage_activation_resolves_a_registered_model_to_its_canonical_wrapper() {
    let directory = tempdir().unwrap();
    let registry_path = directory.path().join("workspace.json");
    let project_id = ProjectId::new_v4();
    let scene_id = SceneId::new_v4();
    let model_id = ModelId::new_v4();
    let repository = directory.path().join("repository");
    let manifest = ProjectManifestV1::new(
        project_id,
        "Model Activation",
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
    let wrapper_path = crate::project::model_wrapper::model_wrapper_path(&repository, model_id);
    fs::create_dir_all(wrapper_path.parent().unwrap()).unwrap();
    fs::write(&wrapper_path, "#usda 1.0\n").unwrap();
    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry.register(project_id, &repository, None).unwrap();
    let service = ProjectApplicationService {
        registry,
        publication_coordinator: ProjectPublicationCoordinator::default(),
        stage_mutations: ProjectStageMutationQueue::default(),
        progress: ProjectImportProgressStore::default(),
        cache_warm: crate::project::cache_warmer::ProjectCacheWarmQueue::default(),
    };

    let target = service
        .resolve_stage_activation(project_id, ProjectStageTarget::Model(model_id))
        .unwrap()
        .expect("a registered Model must resolve to its wrapper");

    assert_eq!(target.target, ProjectStageTarget::Model(model_id));
    assert_eq!(target.path, fs::canonicalize(wrapper_path).unwrap());
    assert!(target.path.is_file());
}
