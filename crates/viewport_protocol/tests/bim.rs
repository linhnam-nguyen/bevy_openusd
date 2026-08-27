use viewport_protocol::{
    BimFieldKey, BimPageRequest, BimSearchQuery, ClassificationLevel, ClassificationRecipe,
    MAX_BIM_REGEX_BYTES,
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

    let recipe = ClassificationRecipe::new(vec![
        ClassificationLevel::new("duplicate", BimFieldKey::Category),
        ClassificationLevel::new("duplicate", BimFieldKey::Family),
    ]);
    assert!(recipe.validate().is_err());
}
