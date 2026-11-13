use super::*;

#[test]
fn default_launch_preserves_the_optional_asset_argument() {
    assert_eq!(
        parse_launch_options(Vec::<String>::new()).unwrap(),
        LaunchOptions::default()
    );
    assert_eq!(
        parse_launch_options(vec!["fixtures/robot.usda".to_owned()]).unwrap(),
        LaunchOptions {
            asset_argument: Some("fixtures/robot.usda".to_owned()),
            transport: None,
            ..Default::default()
        }
    );
}

#[test]
fn legacy_stdio_flags_are_rejected() {
    for arguments in [
        vec!["--stdio".to_owned()],
        vec!["--transport".to_owned(), "stdio".to_owned()],
        vec!["--transport=stdio".to_owned()],
    ] {
        assert!(parse_launch_options(arguments).is_err());
    }
}

#[test]
fn transport_and_asset_can_be_supplied_in_either_order() {
    for arguments in [
        vec!["--webrtc".to_owned(), "fixtures/robot.usda".to_owned()],
        vec![
            "fixtures/robot.usda".to_owned(),
            "--transport".to_owned(),
            "webrtc".to_owned(),
        ],
    ] {
        assert_eq!(
            parse_launch_options(arguments).unwrap(),
            LaunchOptions {
                asset_argument: Some("fixtures/robot.usda".to_owned()),
                transport: Some(ViewportTransport::WebRtc),
                ..Default::default()
            }
        );
    }
}

#[test]
fn codec_selection_is_parsed_without_becoming_an_asset_path() {
    assert_eq!(
        parse_launch_options(vec!["--codec".to_owned(), "av1".to_owned()])
            .unwrap()
            .codec,
        CodecId::Av1
    );
    assert_eq!(
        parse_launch_options(vec!["--codec=h265".to_owned()])
            .unwrap()
            .codec,
        CodecId::H265
    );
}

#[test]
fn benchmark_flags_are_parsed_correctly() {
    let options = parse_launch_options(vec![
        "--benchmark".to_owned(),
        "--benchmark-scenario".to_owned(),
        "S1".to_owned(),
        "--benchmark-warmup-frames".to_owned(),
        "10".to_owned(),
        "--benchmark-frames".to_owned(),
        "50".to_owned(),
        "--benchmark-output".to_owned(),
        "target/out.json".to_owned(),
        "--benchmark-label".to_owned(),
        "baseline".to_owned(),
    ])
    .unwrap();

    assert!(options.benchmark);
    assert_eq!(options.benchmark_scenario, Some("S1".to_string()));
    assert_eq!(options.benchmark_warmup_frames, 10);
    assert_eq!(options.benchmark_frames, 50);
    assert_eq!(
        options.benchmark_output,
        Some("target/out.json".to_string())
    );
    assert_eq!(options.benchmark_label, "baseline");
}

#[test]
fn stream_configuration_flags_select_the_requested_matrix_case() {
    let options = parse_launch_options(vec![
        "--width".to_owned(),
        "1280".to_owned(),
        "--height".to_owned(),
        "720".to_owned(),
        "--fps".to_owned(),
        "120".to_owned(),
    ])
    .unwrap();

    assert_eq!(options.width, 1280);
    assert_eq!(options.height, 720);
    assert_eq!(options.fps, 120);
}

#[test]
fn renderer_matrix_benchmark_flag_selects_the_release_matrix_runner() {
    let options = parse_launch_options(vec!["--benchmark-renderer-matrix".to_owned()]).unwrap();

    assert!(options.benchmark);
    assert!(options.benchmark_renderer_matrix);
}

#[test]
fn mesh_profile_benchmark_flag_enables_profile_mode() {
    let options = parse_launch_options(vec!["--benchmark-mesh-profile".to_owned()]).unwrap();

    assert!(options.benchmark);
    assert!(options.benchmark_mesh_profile);
}
