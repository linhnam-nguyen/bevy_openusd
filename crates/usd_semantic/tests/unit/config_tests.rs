use super::SemanticConfig;

#[test]
fn nvidia_revit_runtime_adapter_uses_observed_element_id_property() {
    let config = SemanticConfig::for_nvidia_revit_connector();

    assert_eq!(
        config.nvidia_revit.identity.element_id_property.as_deref(),
        Some("BIM:Instance:ElementId")
    );
    assert!(config.nvidia_revit.identity.family_name_property.is_none());
    assert_eq!(
        config
            .nvidia_revit
            .classification
            .category_property
            .as_deref(),
        Some("BIM:Instance:Category")
    );
    assert_eq!(
        config
            .nvidia_revit
            .classification
            .type_name_property
            .as_deref(),
        Some("BIM:Type:Name")
    );
    assert!(config.include_custom_properties);
}
