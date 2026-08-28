use viewport_protocol::{
    BimFieldKey, BimPageRequest, BimPropertiesReadModel, BimPropertyGroupId,
    BimPropertyProvenanceReadModel, BimPropertyProvenanceStatus, BimPropertyReadModel,
    BimSearchQuery, ClassificationLevel, ClassificationRecipe, CommonValue, MAX_BIM_REGEX_BYTES,
    MAX_BIM_SEARCH_OFFSET, SceneAnchor, ViewportCommand,
};

#[test]
fn classification_recipe_and_search_queries_validate_as_typed_contracts() {
    let recipe = ClassificationRecipe::new(vec![
        ClassificationLevel::new("category", BimFieldKey::Category),
        ClassificationLevel::new("level", BimFieldKey::property("Level")),
    ]);
    recipe.validate().expect("classification recipe validates");

    let query = BimSearchQuery::ReplacementPreview {
        property: "Mark".to_owned(),
        pattern: "^AHU-(\\d+)$".to_owned(),
        replacement: "MEP-AHU-${1}".to_owned(),
        page: BimPageRequest::new(0, 50),
    };
    query.validate().expect("replacement query validates");
}

#[test]
fn protocol_rejects_unbounded_bim_inputs() {
    let query = BimSearchQuery::PropertyValueRegex {
        pattern: "x".repeat(MAX_BIM_REGEX_BYTES + 1),
        page: BimPageRequest::new(0, 10),
    };
    assert!(query.validate().is_err());

    let query = BimSearchQuery::PropertyValueRegex {
        pattern: "x".to_owned(),
        page: BimPageRequest::new(MAX_BIM_SEARCH_OFFSET + 1, 10),
    };
    assert!(query.validate().is_err());

    let recipe = ClassificationRecipe::new(vec![
        ClassificationLevel::new("duplicate", BimFieldKey::Category),
        ClassificationLevel::new("duplicate", BimFieldKey::Family),
    ]);
    assert!(recipe.validate().is_err());
}

#[test]
fn property_group_identity_and_selection_revision_round_trip() {
    let model = BimPropertiesReadModel {
        targets: Vec::new(),
        selection_revision: 9,
        groups: vec![viewport_protocol::BimPropertyGroupReadModel {
            id: BimPropertyGroupId::SourceFallback,
            name: "<Ungrouped>".to_owned(),
            editable_group: false,
            properties: vec![BimPropertyReadModel {
                key: "Mark".to_owned(),
                group_id: BimPropertyGroupId::SourceFallback,
                value: CommonValue::Multiple,
                target_values: Vec::new(),
                measurement: None,
                units: Vec::new(),
                current_display_unit: None,
                editable: false,
            }],
        }],
    };
    let encoded = serde_json::to_string(&model).expect("BIM properties serialize");
    let decoded: BimPropertiesReadModel =
        serde_json::from_str(&encoded).expect("BIM properties deserialize");

    assert_eq!(decoded, model);
}

#[test]
fn provenance_command_and_read_model_are_typed_and_bounded() {
    let target = SceneAnchor::active_session("/World/Door");
    let command = ViewportCommand::RequestBimPropertyProvenance {
        target: target.clone(),
        property: "Mark".to_owned(),
        history_head: "c10".to_owned(),
    };
    command.validate().expect("provenance command validates");

    let model = BimPropertyProvenanceReadModel {
        target,
        property: "Mark".to_owned(),
        history_head: "c10".to_owned(),
        status: BimPropertyProvenanceStatus::Available,
        commit_id: Some("abc123".to_owned()),
        commit_message: Some("Update door mark".to_owned()),
        author_name: Some("BIM Author".to_owned()),
        author_email: Some("author@example.test".to_owned()),
        authored_at_seconds: Some(1_725_000_000),
        old_value: Some(viewport_protocol::CanonicalValue::Text("D-01".to_owned())),
        new_value: Some(viewport_protocol::CanonicalValue::Text("D-02".to_owned())),
    };
    let encoded = serde_json::to_string(&model).expect("provenance serializes");
    let decoded: BimPropertyProvenanceReadModel =
        serde_json::from_str(&encoded).expect("provenance deserializes");

    assert_eq!(decoded, model);
}
