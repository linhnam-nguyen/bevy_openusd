use viewport_protocol::{
    ColorRgb8, GroundGridOrigin, RenderMode, SamplingPreference, SamplingProvider,
    SamplingReadModel, SceneAnchor, SectionBoxReadModel, SelectionPresentationSettings,
    SelectionReadModel, ViewerEnvironmentSettings, ViewerSettingsCapabilities,
    ViewerSettingsReadModel,
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

#[test]
fn selection_read_model_supports_empty_single_and_many_targets() {
    let empty = SelectionReadModel::default();
    assert!(empty.validate().is_ok());

    let first = SceneAnchor::active_session("/World/First");
    let second = SceneAnchor::active_session("/World/Second");
    let single = SelectionReadModel {
        targets: vec![first.clone()],
        primary: Some(first.clone()),
    };
    assert!(single.validate().is_ok());

    let many = SelectionReadModel {
        targets: vec![second.clone(), first.clone()],
        primary: Some(second),
    };
    assert!(many.validate().is_ok());
    let json = serde_json::to_string(&many).expect("many-target selection serializes");
    let decoded: SelectionReadModel = serde_json::from_str(&json).expect("selection decodes");
    assert_eq!(
        decoded.targets,
        vec![first, SceneAnchor::active_session("/World/Second")]
    );
}

#[test]
fn selection_read_model_rejects_duplicates_and_non_member_primary() {
    let target = SceneAnchor::active_session("/World/Selected");
    assert!(
        SelectionReadModel {
            targets: vec![target.clone(), target.clone()],
            primary: Some(target.clone()),
        }
        .validate()
        .is_err()
    );
    assert!(
        SelectionReadModel {
            targets: vec![target],
            primary: Some(SceneAnchor::active_session("/World/Other")),
        }
        .validate()
        .is_err()
    );
}

#[test]
fn selection_read_model_accepts_legacy_single_target_and_serializes_canonically() {
    let decoded: SelectionReadModel = serde_json::from_str(
        r#"{"target":{"session_id":null,"prim_path":"/World/Legacy","instance_context":null}}"#,
    )
    .expect("legacy target decodes");
    let target = SceneAnchor::active_session("/World/Legacy");
    assert_eq!(decoded.targets, vec![target.clone()]);
    assert_eq!(decoded.primary, Some(target));

    let json = serde_json::to_string(&decoded).expect("selection serializes");
    assert!(json.contains("\"targets\""));
    assert!(!json.contains("\"target\""));
}

#[test]
fn selection_commands_validate_membership_and_anchor_identity() {
    let target = SceneAnchor::active_session("/World/Selected");
    assert!(
        viewport_protocol::ViewportCommand::ReplaceSelection {
            targets: vec![target.clone()],
            primary: Some(target),
        }
        .validate()
        .is_ok()
    );
    assert!(
        viewport_protocol::ViewportCommand::ReplaceSelection {
            targets: vec![SceneAnchor::active_session("/World/Selected")],
            primary: Some(SceneAnchor::active_session("/World/Other")),
        }
        .validate()
        .is_err()
    );
    assert!(
        viewport_protocol::ViewportCommand::AddSelectionTarget {
            target: SceneAnchor::active_session("not-a-usd-path"),
            make_primary: true,
        }
        .validate()
        .is_err()
    );
}

#[test]
fn viewer_settings_commands_are_typed_and_do_not_expose_provider_or_geometry_internals() {
    let environment = ViewerEnvironmentSettings {
        render_mode: RenderMode::UniformColor,
        shadows_enabled: false,
        grid_visible: true,
        grid_color: ColorRgb8::new(0x6B, 0x72, 0x80),
        grid_origin: GroundGridOrigin::LoadedScene,
        background_color: ColorRgb8::new(0x11, 0x18, 0x27),
        default_surface_color: ColorRgb8::new(0x9C, 0xA3, 0xAF),
    };
    let selection = SelectionPresentationSettings {
        boundary_enabled: true,
        boundary_color: ColorRgb8::new(0xFA, 0xCC, 0x15),
        color_change_enabled: false,
        selection_color: ColorRgb8::new(0x38, 0xBD, 0xF8),
        hover_color_change_enabled: false,
        hover_color: ColorRgb8::new(0x7D, 0xD3, 0xFC),
    };
    let commands = [
        viewport_protocol::ViewportCommand::SetEnvironmentSettings {
            settings: environment,
        },
        viewport_protocol::ViewportCommand::SetSamplingPreference {
            preference: SamplingPreference { enabled: true },
        },
        viewport_protocol::ViewportCommand::SetSelectionPresentationSettings {
            settings: selection,
        },
        viewport_protocol::ViewportCommand::SetSectionBox { enabled: true },
    ];

    for command in commands {
        assert!(command.validate().is_ok());
        let json = serde_json::to_string(&command).expect("typed settings command serializes");
        let decoded: viewport_protocol::ViewportCommand =
            serde_json::from_str(&json).expect("typed settings command decodes");
        assert_eq!(decoded, command);
        assert!(!json.contains("json"));
        assert!(!json.contains("provider"));
        assert!(!json.contains("transform"));
    }
}

#[test]
fn authoritative_viewer_settings_read_model_round_trips_capabilities_and_section_targets() {
    let selected = SceneAnchor::active_session("/World/Selected");
    let settings = ViewerSettingsReadModel {
        environment: ViewerEnvironmentSettings::default(),
        sampling: SamplingReadModel {
            preference: SamplingPreference { enabled: true },
            provider: SamplingProvider::None,
        },
        selection: SelectionPresentationSettings::default(),
        section_box: SectionBoxReadModel {
            enabled: true,
            targets: vec![selected],
        },
        capabilities: ViewerSettingsCapabilities {
            ray_traced_supported: false,
            dlss_available: false,
            fsr_available: false,
        },
    };

    let json = serde_json::to_string(&settings).expect("viewer settings serialize");
    let decoded: ViewerSettingsReadModel =
        serde_json::from_str(&json).expect("viewer settings deserialize");

    assert_eq!(decoded, settings);
    assert!(json.contains("section_box"));
    assert!(json.contains("capabilities"));
}
