use viewport_protocol::{
    BimFieldKey, BimPageRequest, BimPropertiesReadModel, BimPropertyGroupId, BimPropertyReadModel,
    BimSearchQuery, ClassificationLevel, ClassificationRecipe, CommonValue, MAX_BIM_REGEX_BYTES,
    MAX_BIM_SEARCH_OFFSET,
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
        properties: vec![BimPropertyReadModel {
            key: "Mark".to_owned(),
            group_id: BimPropertyGroupId::SourceFallback,
            value: CommonValue::Multiple,
            target_values: Vec::new(),
            measurement: None,
            units: Vec::new(),
            editable: false,
        }],
    };
    let encoded = serde_json::to_string(&model).expect("BIM properties serialize");
    let decoded: BimPropertiesReadModel =
        serde_json::from_str(&encoded).expect("BIM properties deserialize");

    assert_eq!(decoded, model);
}
