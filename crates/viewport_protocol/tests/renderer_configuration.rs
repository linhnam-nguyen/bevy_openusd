use viewport_protocol::{
    ProtocolValidationError, RenderMode, RendererConfiguration, ViewportCommand,
    ViewportCommandEnvelope, ViewportWireMessage, decode_json_line, encode_json_line,
};

#[test]
fn renderer_configuration_defaults_are_explicit() {
    let configuration = RendererConfiguration::default();

    assert!(configuration.grid);
    assert!(configuration.shadows);
    assert!(!configuration.edges);
    assert_eq!(configuration.render_mode, RenderMode::Shaded);
    assert_eq!(configuration.preferred_fps, Some(60));
    configuration.validate().unwrap();
}

#[test]
fn renderer_configuration_round_trips_with_an_uncapped_fps() {
    let configuration = RendererConfiguration {
        grid: false,
        shadows: false,
        edges: true,
        render_mode: RenderMode::Wireframe,
        preferred_fps: None,
    };

    let json = serde_json::to_string(&configuration).expect("renderer config serializes");
    let decoded: RendererConfiguration =
        serde_json::from_str(&json).expect("renderer config deserializes");

    assert_eq!(decoded, configuration);
    assert!(json.contains("wireframe"));
    assert!(json.contains("preferred_fps"));
}

#[test]
fn renderer_configuration_rejects_out_of_range_fps() {
    for preferred_fps in [Some(0), Some(241)] {
        let configuration = RendererConfiguration {
            preferred_fps,
            ..Default::default()
        };

        assert!(matches!(
            configuration.validate(),
            Err(ProtocolValidationError::InvalidFrameRate { value }) if value == preferred_fps.unwrap()
        ));
    }
}

#[test]
fn unsupported_render_mode_is_an_explicit_decode_error() {
    let error = serde_json::from_str::<RendererConfiguration>(
        r#"{"grid":true,"shadows":true,"edges":false,"render_mode":"flat","preferred_fps":60}"#,
    )
    .expect_err("unsupported modes must not silently fall back");

    assert!(error.to_string().contains("unknown variant"));
}

#[test]
fn typed_renderer_command_preserves_request_correlation() {
    let message = ViewportWireMessage::Command(ViewportCommandEnvelope::new(
        "renderer-42",
        ViewportCommand::SetRendererConfiguration {
            configuration: RendererConfiguration::default(),
        },
    ));

    if let ViewportWireMessage::Command(command) = &message {
        command.validate().unwrap();
    } else {
        unreachable!("message is constructed as a command");
    }

    let encoded = encode_json_line(&message).expect("renderer command serializes");
    let decoded = decode_json_line(&encoded).expect("renderer command deserializes");

    assert_eq!(decoded, message);
    let ViewportWireMessage::Command(command) = decoded else {
        unreachable!("decoded message is constructed as a command");
    };
    assert_eq!(command.request_id, "renderer-42");
}
