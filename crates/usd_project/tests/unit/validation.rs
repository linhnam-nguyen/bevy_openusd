#[cfg(test)]
mod tests {
    use crate::{
        ModelId, ModelManifestEntry, ModelSourceKind, ProjectId, ProjectManifestError,
        ProjectManifestV1, ProjectRoot, SceneId, SceneManifestEntry, SceneMemberId, StorageKey,
    };

    fn storage_key(value: &str) -> StorageKey {
        StorageKey::new(value).unwrap()
    }

    #[test]
    fn identity_categories_are_checked_by_the_type_system() {
        fn accept_project(_: ProjectId) {}
        fn accept_scene(_: SceneId) {}
        fn accept_member(_: SceneMemberId) {}
        fn accept_model(_: ModelId) {}

        accept_project(ProjectId::new_v4());
        accept_scene(SceneId::new_v4());
        accept_member(SceneMemberId::new_v4());
        accept_model(ModelId::new_v4());
    }

    #[test]
    fn every_project_root_variant_round_trips() {
        let roots = [
            ProjectRoot::Empty,
            ProjectRoot::Model(ModelId::new_v4()),
            ProjectRoot::Scene(SceneId::new_v4()),
        ];

        for root in roots {
            let encoded = serde_json::to_string(&root).unwrap();
            let decoded: ProjectRoot = serde_json::from_str(&encoded).unwrap();
            assert_eq!(root, decoded);
        }
    }

    #[test]
    fn source_kind_validation_preserves_non_empty_metadata() {
        let source = ModelSourceKind::external("ifc").unwrap();

        assert_eq!(source, ModelSourceKind::External("ifc".to_owned()));
        assert!(source.validate().is_ok());
        assert!(
            ModelSourceKind::External("\n".to_owned())
                .validate()
                .is_err()
        );
    }

    #[test]
    fn manifest_validation_rejects_duplicate_ids_and_storage_keys() {
        let scene_id = SceneId::new_v4();
        let manifest = ProjectManifestV1::new(
            ProjectId::new_v4(),
            "Project",
            ProjectRoot::Empty,
            vec![
                SceneManifestEntry {
                    id: scene_id,
                    storage_key: storage_key("scene-a"),
                    display_name: "Scene A".to_owned(),
                },
                SceneManifestEntry {
                    id: scene_id,
                    storage_key: storage_key("scene-b"),
                    display_name: "Scene B".to_owned(),
                },
            ],
            Vec::new(),
        );
        assert!(matches!(
            manifest.validate(),
            Err(ProjectManifestError::DuplicateSceneId { id }) if id == scene_id
        ));

        let shared_key = storage_key("shared");
        let manifest = ProjectManifestV1::new(
            ProjectId::new_v4(),
            "Project",
            ProjectRoot::Empty,
            vec![SceneManifestEntry {
                id: SceneId::new_v4(),
                storage_key: shared_key.clone(),
                display_name: "Scene".to_owned(),
            }],
            vec![ModelManifestEntry {
                id: ModelId::new_v4(),
                source_kind: ModelSourceKind::Usd,
                storage_key: shared_key,
                display_name: "Model".to_owned(),
            }],
        );
        assert!(matches!(
            manifest.validate(),
            Err(ProjectManifestError::DuplicateStorageKey { value }) if value == "shared"
        ));
    }

    #[test]
    fn manifest_validation_rejects_missing_root_and_empty_name() {
        let missing_scene = SceneId::new_v4();
        let manifest = ProjectManifestV1::new(
            ProjectId::new_v4(),
            "Project",
            ProjectRoot::Scene(missing_scene),
            Vec::new(),
            Vec::new(),
        );
        assert!(matches!(
            manifest.validate(),
            Err(ProjectManifestError::MissingRootScene { id }) if id == missing_scene
        ));

        let manifest = ProjectManifestV1::new(
            ProjectId::new_v4(),
            "  ",
            ProjectRoot::Empty,
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(
            manifest.validate(),
            Err(ProjectManifestError::EmptyProjectName)
        );
    }

    #[test]
    fn manifest_validation_builds_indexed_lookup_and_allows_empty_root() {
        let scene_id = SceneId::new_v4();
        let model_id = ModelId::new_v4();
        let manifest = ProjectManifestV1::new(
            ProjectId::new_v4(),
            "Project",
            ProjectRoot::Empty,
            vec![SceneManifestEntry {
                id: scene_id,
                storage_key: storage_key("scene"),
                display_name: "Scene".to_owned(),
            }],
            vec![ModelManifestEntry {
                id: model_id,
                source_kind: ModelSourceKind::Usd,
                storage_key: storage_key("model"),
                display_name: "Model".to_owned(),
            }],
        );

        let validated = manifest.validate_and_index().unwrap();

        assert_eq!(validated.scene(scene_id).unwrap().id, scene_id);
        assert_eq!(validated.model(model_id).unwrap().id, model_id);
        assert!(validated.scene(SceneId::new_v4()).is_none());
        assert!(validated.model(ModelId::new_v4()).is_none());
        assert_eq!(validated.scenes().len(), 1);
        assert_eq!(validated.models().len(), 1);
    }
}
