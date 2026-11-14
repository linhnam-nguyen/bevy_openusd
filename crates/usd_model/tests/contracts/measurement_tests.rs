use super::*;

#[test]
fn metadata_round_trips_without_losing_source_unit() {
    let original = MeasurementMetadata::new("length", "m", Some("ft".to_owned()));
    let encoded = serde_json::to_string(&original).expect("measurement metadata serializes");
    let decoded: MeasurementMetadata =
        serde_json::from_str(&encoded).expect("measurement metadata deserializes");

    assert_eq!(decoded, original);
    assert_eq!(decoded.quantity.as_str(), "length");
    assert_eq!(decoded.canonical_unit.as_str(), "m");
    assert_eq!(decoded.source_unit.as_ref().map(UnitId::as_str), Some("ft"));
}

#[test]
fn unknown_source_unit_is_distinct_from_a_known_unit() {
    let known = MeasurementMetadata::new("length", "m", Some("m".to_owned()));
    let unknown = MeasurementMetadata::new("length", "m", None::<String>);

    assert_ne!(known, unknown);
}

#[test]
fn legacy_property_json_defaults_measurement_to_none() {
    let property: crate::SemanticProperty =
        serde_json::from_str(r#"{"name":"legacy_height","value":{"Real":10.0}}"#)
            .expect("legacy semantic property deserializes");

    assert_eq!(property.name, "legacy_height");
    assert_eq!(property.measurement, None);
}
