use viewport_protocol::*;

fn row(value: CanonicalValue) -> BimReplacementPreviewRow {
    BimReplacementPreviewRow {
        anchor: SceneAnchor::active_session("/World/Door"),
        label: "Door".to_owned(),
        property: "Mark".to_owned(),
        old_value: "A".to_owned(),
        proposed_value: "B".to_owned(),
        expected_old_value: CanonicalValue::Text("A".to_owned()),
        proposed_canonical_value: Some(value),
        measurement: None,
    }
}

#[test]
fn proposed_editor_value_preserves_json_scalar_shape() {
    assert_eq!(
        row(CanonicalValue::Integer(4)).proposed_editor_value(),
        Some(EditorValue::from(4))
    );
    assert_eq!(
        row(CanonicalValue::Text("B".to_owned())).proposed_editor_value(),
        Some(EditorValue::String("B".to_owned()))
    );
    assert_eq!(
        row(CanonicalValue::Json("{\"ok\":true}".to_owned())).proposed_editor_value(),
        Some(serde_json::json!({"ok": true}))
    );
}
