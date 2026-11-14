use super::*;

fn property(name: &str, value: CanonicalValue) -> SemanticProperty {
    SemanticProperty {
        name: name.to_owned(),
        value,
        measurement: None,
        display_name: None,
    }
}

#[test]
fn mapping_normalizes_numeric_value_and_attaches_metadata() {
    let mut properties = vec![
        property("height", CanonicalValue::Real(10.0)),
        property("height_unit", CanonicalValue::Text("[ft_i]".to_owned())),
    ];
    let config = NvidiaRevitConfig {
        classification: NvidiaRevitClassificationConfig::default(),
        measurement_mappings: vec![NvidiaRevitMeasurementMapping::new(
            "height",
            "length",
            "height_unit",
        )],
        ..Default::default()
    };

    attach_measurements(&mut properties, &config);

    assert_eq!(properties[0].value, CanonicalValue::Real(3.048));
    let metadata = properties[0]
        .measurement
        .as_ref()
        .expect("configured measurement");
    assert_eq!(metadata.quantity.as_str(), "length");
    assert_eq!(metadata.canonical_unit.as_str(), "m");
    assert_eq!(metadata.source_unit.as_ref().unwrap().as_str(), "[ft_i]");
}

#[test]
fn missing_or_unknown_units_preserve_the_typed_value_without_guessing() {
    let mut missing = vec![property("height", CanonicalValue::Real(10.0))];
    let mut unknown = vec![
        property("height", CanonicalValue::Real(10.0)),
        property(
            "height_unit",
            CanonicalValue::Text("revit:unknown".to_owned()),
        ),
    ];
    let config = NvidiaRevitConfig {
        classification: NvidiaRevitClassificationConfig::default(),
        measurement_mappings: vec![NvidiaRevitMeasurementMapping::new(
            "height",
            "length",
            "height_unit",
        )],
        ..Default::default()
    };

    attach_measurements(&mut missing, &config);
    attach_measurements(&mut unknown, &config);

    assert_eq!(missing[0].value, CanonicalValue::Real(10.0));
    assert_eq!(unknown[0].value, CanonicalValue::Real(10.0));
    assert!(missing[0].measurement.is_none());
    assert!(unknown[0].measurement.is_none());
}

#[test]
fn identity_mapping_does_not_promote_category_to_family_name() {
    let properties = vec![
        property(
            "BIM:Instance:Category",
            CanonicalValue::Text("Murs".to_owned()),
        ),
        property(
            "BIM:Instance:ElementId",
            CanonicalValue::Text("150663".to_owned()),
        ),
        property(
            "BIM:Type:Name",
            CanonicalValue::Text("Générique - 200 mm".to_owned()),
        ),
    ];
    let identity = extract_bim_identity(
        &properties,
        &NvidiaRevitIdentityConfig {
            element_id_property: Some("BIM:Instance:ElementId".to_owned()),
            family_name_property: None,
        },
    );

    assert_eq!(identity.element_id.as_deref(), Some("150663"));
    assert_eq!(identity.family_name, None);
}

#[test]
fn classification_mapping_keeps_generic_usd_fields_separate() {
    let properties = vec![
        property(
            "BIM:Instance:Category",
            CanonicalValue::Text("Murs".to_owned()),
        ),
        property(
            "BIM:Type:Name",
            CanonicalValue::Text("Générique - 400 mm".to_owned()),
        ),
    ];
    let classification = extract_bim_classification(
        &properties,
        &NvidiaRevitClassificationConfig {
            category_property: Some("BIM:Instance:Category".to_owned()),
            type_name_property: Some("BIM:Type:Name".to_owned()),
            ..Default::default()
        },
    );

    assert_eq!(classification.category.as_deref(), Some("Murs"));
    assert_eq!(
        classification.type_name.as_deref(),
        Some("Générique - 400 mm")
    );
    assert_eq!(
        classification.category_property.as_deref(),
        Some("BIM:Instance:Category")
    );
}

#[test]
fn identity_mapping_accepts_only_explicit_family_name_source() {
    let properties = vec![property(
        "BIM:Instance:FamilyName",
        CanonicalValue::Text("Basic Wall".to_owned()),
    )];
    let identity = extract_bim_identity(
        &properties,
        &NvidiaRevitIdentityConfig {
            family_name_property: Some("BIM:Instance:FamilyName".to_owned()),
            ..Default::default()
        },
    );

    assert_eq!(identity.family_name.as_deref(), Some("Basic Wall"));
}

#[test]
fn property_descriptor_precedence_authored_display_name_over_suffix() {
    let with_authored = nvidia_revit_property_descriptor_with_display_name(
        "BIM:Type:tn__Codedassemblage_pFW3",
        Some("Code d'assemblage"),
    );
    assert_eq!(with_authored.label, "Code d'assemblage");
    assert_eq!(with_authored.scope, BimPropertyScope::Type);

    let instance_authored = nvidia_revit_property_descriptor_with_display_name(
        "BIM:Instance:tn__Extensioninfrieure_xIYa3",
        Some("Extension inférieure"),
    );
    assert_eq!(instance_authored.label, "Extension inférieure");
    assert_eq!(instance_authored.scope, BimPropertyScope::Instance);

    let fallback =
        nvidia_revit_property_descriptor_with_display_name("BIM:Instance:ElementId", None);
    assert_eq!(fallback.label, "ElementId");
    assert_eq!(fallback.scope, BimPropertyScope::Instance);

    let whitespace_fallback =
        nvidia_revit_property_descriptor_with_display_name("BIM:Instance:IfcGUID", Some("   "));
    assert_eq!(whitespace_fallback.label, "IfcGUID");
    assert_eq!(whitespace_fallback.scope, BimPropertyScope::Instance);

    let other = nvidia_revit_property_descriptor("custom_prop");
    assert_eq!(other.label, "custom_prop");
    assert_eq!(other.scope, BimPropertyScope::Other);
}
