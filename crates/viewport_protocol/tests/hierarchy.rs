use viewport_protocol::{
    HierarchyNodeId, HierarchyNodeReadModel, HierarchyReadModel, HierarchySource,
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
    assert_eq!(
        leaf.anchor.as_ref().map(|anchor| anchor.prim_path.as_str()),
        Some("/World/Wall")
    );
    assert_eq!(leaf.parent_id, Some(group.id));
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
