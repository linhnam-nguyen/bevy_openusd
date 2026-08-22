use viewport_protocol::{
    ColorRgb8, GroundGridOrigin, RenderMode, SamplingPreference, SamplingProvider,
    SelectionPresentationSettings, ViewerEnvironmentSettings,
};

#[test]
fn environment_settings_round_trip_with_explicit_wire_values() {
    let settings = ViewerEnvironmentSettings {
        render_mode: RenderMode::RayTraced,
        shadows_enabled: true,
        grid_visible: false,
        grid_color: ColorRgb8::new(0x12, 0x34, 0x56),
        grid_origin: GroundGridOrigin::WorldOrigin,
        background_color: ColorRgb8::new(0x20, 0x30, 0x40),
        default_surface_color: ColorRgb8::new(0xA0, 0xB0, 0xC0),
    };

    let json = serde_json::to_string(&settings).expect("environment settings serialize");
    let decoded: ViewerEnvironmentSettings =
        serde_json::from_str(&json).expect("environment settings deserialize");

    assert_eq!(decoded, settings);
    assert!(json.contains("ray_traced"));
    assert!(json.contains("world_origin"));
}

#[test]
fn rgb_channels_reject_values_outside_u8_wire_range() {
    for json in [
        r#"{"r":256,"g":0,"b":0}"#,
        r#"{"r":-1,"g":0,"b":0}"#,
        r#"{"r":1.5,"g":0,"b":0}"#,
    ] {
        assert!(serde_json::from_str::<ColorRgb8>(json).is_err(), "{json}");
    }
}

#[test]
fn selection_presentation_settings_round_trip_without_renderer_types() {
    let settings = SelectionPresentationSettings {
        boundary_enabled: true,
        boundary_color: ColorRgb8::new(255, 0, 0),
        color_change_enabled: false,
        selection_color: ColorRgb8::new(0, 255, 0),
        hover_color_change_enabled: true,
        hover_color: ColorRgb8::new(0, 0, 255),
    };

    let encoded = serde_json::to_string(&settings).expect("selection settings serialize");
    let decoded: SelectionPresentationSettings =
        serde_json::from_str(&encoded).expect("selection settings deserialize");

    assert_eq!(decoded, settings);
}

#[test]
fn sampling_intent_and_provider_are_separate_wire_values() {
    let preference = SamplingPreference { enabled: true };
    let preference_json =
        serde_json::to_string(&preference).expect("sampling preference serialize");
    let provider_json =
        serde_json::to_string(&SamplingProvider::Dlss).expect("sampling provider serialize");

    assert_eq!(preference_json, r#"{"enabled":true}"#);
    assert_eq!(provider_json, "\"dlss\"");
    assert_eq!(SamplingProvider::default(), SamplingProvider::None);
}

#[test]
fn protocol_v1_legacy_surface_remains_unchanged() {
    assert_eq!(viewport_protocol::PROTOCOL_VERSION, 1);
    let decoded: SamplingProvider = serde_json::from_str("\"fsr\"").unwrap();
    assert_eq!(decoded, SamplingProvider::Fsr);
}
