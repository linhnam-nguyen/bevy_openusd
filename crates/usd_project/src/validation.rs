#[cfg(test)]
mod tests {
    use crate::{ModelId, ModelSourceKind, ProjectId, ProjectRoot, SceneId, SceneMemberId};

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
}
