use super::*;

#[test]
fn benchmark_identity_records_git_and_glacial_sha() {
    let identity = BenchmarkIdentity::new(
        "M1-C1",
        "hummingbird.usdz",
        Some("S1".into()),
        "Apple M4".into(),
        1920,
        1080,
        60.0,
    );
    assert_eq!(identity.checkpoint_id, "M1-C1");
    assert_eq!(identity.scene_label, "hummingbird.usdz");
    assert_eq!(identity.scenario_code, Some("S1".into()));
    assert_eq!(identity.gpu_adapter, "Apple M4");
    assert!(!identity.git_sha.is_empty());
    assert!(!identity.glacial_sha.is_empty());
    assert_eq!(identity.width, 1920);
    assert_eq!(identity.height, 1080);
    assert_eq!(identity.requested_fps, 60.0);
}

#[test]
fn render_configuration_defaults_and_round_trip() {
    let config = RenderConfiguration::default();
    assert!(config.grid);
    assert!(config.shadows);
    assert!(!config.edges);
    assert_eq!(config.render_mode, RenderMode::Shaded);

    let json = serde_json::to_string(&config).expect("must serialize");
    let deserialized: RenderConfiguration = serde_json::from_str(&json).expect("must deserialize");
    assert_eq!(config, deserialized);
}
