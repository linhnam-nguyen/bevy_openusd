use openusd::sdf::Value;
use openusd::usd::Stage;
use usd_model::{CanonicalValue, MeasurementMetadata, UnitId};
use viewport_protocol::SceneAnchor;

use super::*;

fn stage_with_attribute(type_name: &str, custom: bool) -> Stage {
    let stage = Stage::builder()
        .in_memory("bim_authoring_locator.usda")
        .expect("stage opens");
    stage
        .define_prim("/World/Wall")
        .expect("prim defines")
        .set_type_name("Xform")
        .expect("prim type authors");
    stage
        .prim(openusd::sdf::path("/World/Wall").expect("path parses"))
        .create_attribute("Width", type_name)
        .expect("attribute creates")
        .set_custom(custom)
        .expect("custom flag authors")
        .set(Value::Double(1.0))
        .expect("value authors");
    stage
}

fn target() -> SceneAnchor {
    SceneAnchor::active_session("/World/Wall")
}

#[test]
fn resolves_editable_custom_attribute_by_stable_key() {
    let stage = stage_with_attribute("double", true);
    let locator =
        resolve_bim_authoring_locator(&stage, &target(), "Width").expect("custom scalar resolves");

    assert_eq!(locator.property_key, "Width");
    assert_eq!(locator.attribute_path, "/World/Wall.Width");
    assert_eq!(locator.type_name.as_deref(), Some("double"));
    assert!(locator.is_editable());
}

#[test]
fn derived_semantic_property_is_explicitly_non_editable() {
    let stage = stage_with_attribute("double", true);
    let locator = resolve_bim_authoring_locator(&stage, &target(), "semantic.category")
        .expect("derived field resolves as a capability descriptor");

    assert_eq!(
        locator.editability,
        BimEditability::NonEditable {
            reason: BimNonEditableReason::DerivedProperty
        }
    );
    assert!(!locator.is_editable());
}

#[test]
fn missing_attribute_is_rejected_without_guessing_a_target() {
    let stage = stage_with_attribute("double", true);
    assert!(matches!(
        resolve_bim_authoring_locator(&stage, &target(), "Missing"),
        Err(BimAuthoringError::AttributeNotFound { .. })
    ));
}

#[test]
fn non_custom_and_unsupported_attributes_are_not_editable() {
    let non_custom = stage_with_attribute("double", false);
    let locator = resolve_bim_authoring_locator(&non_custom, &target(), "Width")
        .expect("schema attribute resolves");
    assert_eq!(
        locator.editability,
        BimEditability::NonEditable {
            reason: BimNonEditableReason::NonCustomAttribute
        }
    );

    let unsupported = stage_with_attribute("dictionary", true);
    let locator = resolve_bim_authoring_locator(&unsupported, &target(), "Width")
        .expect("unsupported attribute resolves");
    assert_eq!(
        locator.editability,
        BimEditability::NonEditable {
            reason: BimNonEditableReason::UnsupportedType
        }
    );
}

#[test]
fn measured_edit_converts_input_to_the_authored_source_unit() {
    let stage = stage_with_attribute("double", true);
    let locator = resolve_bim_authoring_locator(&stage, &target(), "Width")
        .expect("measured attribute resolves");
    let measurement = MeasurementMetadata::new("length", "m", Some("mm".to_owned()));

    let (authored, canonical) = prepare_bim_value(
        &locator,
        &serde_json::json!(0.2),
        Some(&UnitId::new("m")),
        Some(&measurement),
    )
    .expect("metres convert to source millimetres");
    assert!(matches!(authored, Value::Double(value) if (value - 200.0).abs() < 1e-9));
    assert_eq!(canonical, CanonicalValue::Real(0.2));

    let current = canonical_value_for_comparison(Value::Double(200.0), Some(&measurement))
        .expect("source value normalizes to canonical metres");
    assert_eq!(current, CanonicalValue::Real(0.2));
}

#[test]
fn measured_edit_rejects_unknown_or_wrong_quantity_units() {
    let stage = stage_with_attribute("double", true);
    let locator = resolve_bim_authoring_locator(&stage, &target(), "Width")
        .expect("measured attribute resolves");
    let measurement = MeasurementMetadata::new("length", "m", Some("mm".to_owned()));

    assert!(matches!(
        prepare_bim_value(
            &locator,
            &serde_json::json!(1.0),
            Some(&UnitId::new("unknown")),
            Some(&measurement),
        ),
        Err(BimAuthoringError::InvalidUnit(_))
    ));
    assert!(matches!(
        prepare_bim_value(
            &locator,
            &serde_json::json!(1.0),
            Some(&UnitId::new("Pa")),
            Some(&measurement),
        ),
        Err(BimAuthoringError::InvalidUnit(_))
    ));
}
