use viewport_protocol::{
    BimFieldKey, ClassificationColorIntent, ClassificationColorSource, ClassificationLevel,
    ClassificationRecipe, HierarchyNodeId, HierarchyNodeKind, HierarchyNodeReadModel,
    HierarchyReadModel, HierarchySource, ViewportCommand,
};

#[test]
fn generic_hierarchy_keeps_virtual_and_scene_identity_distinct() {
    let group = HierarchyNodeReadModel::virtual_node(
        HierarchyNodeId::new("bim-group-category-walls"),
        None,
        "Walls".to_owned(),
        "Classification / Walls".to_owned(),
        true,
    );
    let leaf = HierarchyNodeReadModel::scene(
        HierarchyNodeId::new("prim-world-wall"),
        Some(group.id.clone()),
        "element-42-Wall".to_owned(),
        "Classification / Walls / element-42-Wall".to_owned(),
        viewport_protocol::SceneAnchor::active_session("/World/Wall"),
        None,
        true,
        false,
    );

    assert!(group.anchor.is_none());
    assert_eq!(group.kind, HierarchyNodeKind::Group);
    assert!(!group.selectable);
    assert_eq!(
        leaf.anchor.as_ref().map(|anchor| anchor.prim_path.as_str()),
        Some("/World/Wall")
    );
    assert_eq!(leaf.kind, HierarchyNodeKind::Object);
    assert!(leaf.selectable);
    assert_eq!(leaf.parent_id, Some(group.id));
}

#[test]
fn generic_hierarchy_can_carry_explicit_virtual_kind_and_selection_policy() {
    let object = HierarchyNodeReadModel::virtual_node_with_kind(
        HierarchyNodeId::new("bim-object"),
        None,
        "Object".to_owned(),
        "Object".to_owned(),
        HierarchyNodeKind::Object,
        true,
        false,
    );

    assert_eq!(object.kind, HierarchyNodeKind::Object);
    assert!(object.selectable);
    assert!(object.anchor.is_none());
}

#[test]
fn generic_hierarchy_read_model_round_trips_provider_and_revision() {
    let model = HierarchyReadModel {
        source: HierarchySource::BimClassification,
        revision: 7,
        nodes: vec![HierarchyNodeReadModel::virtual_node(
            HierarchyNodeId::new("bim-root"),
            None,
            "Classification".to_owned(),
            "Classification".to_owned(),
            true,
        )],
    };
    let encoded = serde_json::to_string(&model).expect("hierarchy model serializes");
    let decoded: HierarchyReadModel =
        serde_json::from_str(&encoded).expect("hierarchy model deserializes");

    assert_eq!(decoded, model);
}

#[test]
fn generic_hierarchy_commands_validate_provider_ids_and_bounds() {
    let command = ViewportCommand::RequestHierarchyChildren {
        source: HierarchySource::Prim,
        parent_id: Some(HierarchyNodeId::new("prim:/World:single")),
        page: 0,
        page_size: 64,
    };
    assert!(command.validate().is_ok());
    assert!(
        ViewportCommand::SearchHierarchy {
            source: HierarchySource::Prim,
            query: "wall".to_owned(),
            offset: 0,
            limit: 30,
        }
        .validate()
        .is_ok()
    );

    assert!(
        ViewportCommand::SetHierarchySource {
            source: HierarchySource::BimClassification,
            classification_recipe: Some(ClassificationRecipe::new(vec![ClassificationLevel::new(
                "category",
                BimFieldKey::Category,
            )])),
        }
        .validate()
        .is_ok()
    );
    assert!(
        ViewportCommand::SetHierarchySource {
            source: HierarchySource::BimClassification,
            classification_recipe: None,
        }
        .validate()
        .is_err()
    );
    assert!(
        ViewportCommand::SetHierarchySource {
            source: HierarchySource::Prim,
            classification_recipe: Some(ClassificationRecipe::new(vec![ClassificationLevel::new(
                "category",
                BimFieldKey::Category,
            )])),
        }
        .validate()
        .is_err()
    );
    assert!(
        ViewportCommand::SetBimClassificationRecipe {
            recipe: Some(ClassificationRecipe::new(vec![ClassificationLevel::new(
                "category",
                BimFieldKey::Category,
            )])),
        }
        .validate()
        .is_ok()
    );
    assert!(
        ViewportCommand::SetBimClassificationRecipe { recipe: None }
            .validate()
            .is_ok()
    );
}

#[test]
fn classification_color_plan_validates_backend_owned_intent() {
    assert!(
        ViewportCommand::SetClassificationColorPlan {
            intent: ClassificationColorIntent {
                source: ClassificationColorSource::Profile("default".to_owned()),
                active_level: Some("category".to_owned()),
                generation: 4,
            },
        }
        .validate()
        .is_ok()
    );

    assert!(
        ViewportCommand::SetClassificationColorPlan {
            intent: ClassificationColorIntent {
                source: ClassificationColorSource::Auto,
                active_level: Some("   ".to_owned()),
                generation: 4,
            },
        }
        .validate()
        .is_err()
    );

    assert!(
        ViewportCommand::SetClassificationColorPlan {
            intent: ClassificationColorIntent {
                source: ClassificationColorSource::Profile(String::new()),
                active_level: None,
                generation: 4,
            },
        }
        .validate()
        .is_err()
    );
}
