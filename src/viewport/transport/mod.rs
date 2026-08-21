//! Render-server transport selection and adapters for the viewport process.

use viewport_protocol::CodecId;

pub(crate) mod frame_capture;
pub(crate) mod webrtc;

pub(crate) use frame_capture::FrameCapturePlugin;

/// The delivered viewport transport enabled for this launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewportTransport {
    /// Headless WebRTC streaming server.
    WebRtc,
}

/// Command-line configuration that is independent of Bevy startup.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LaunchOptions {
    pub(crate) asset_argument: Option<String>,
    pub(crate) transport: Option<ViewportTransport>,
    pub(crate) headless: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) fps: u32,
    pub(crate) codec: CodecId,
    pub(crate) benchmark: bool,
    pub(crate) benchmark_renderer_matrix: bool,
    pub(crate) benchmark_scenario: Option<String>,
    pub(crate) benchmark_warmup_frames: u64,
    pub(crate) benchmark_frames: u64,
    pub(crate) benchmark_output: Option<String>,
    pub(crate) benchmark_label: String,
    pub(crate) benchmark_client_ready_file: Option<String>,
    pub(crate) benchmark_measurement_start_file: Option<String>,
    pub(crate) benchmark_measurement_idle_file: Option<String>,
    pub(crate) benchmark_measurement_complete_file: Option<String>,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            asset_argument: None,
            transport: None,
            headless: false,
            width: 1920,
            height: 1080,
            fps: 60,
            codec: CodecId::H264,
            benchmark: false,
            benchmark_renderer_matrix: false,
            benchmark_scenario: None,
            benchmark_warmup_frames: 30,
            benchmark_frames: 120,
            benchmark_output: None,
            benchmark_label: "baseline".to_string(),
            benchmark_client_ready_file: None,
            benchmark_measurement_start_file: None,
            benchmark_measurement_idle_file: None,
            benchmark_measurement_complete_file: None,
        }
    }
}

/// Parses `usdview` arguments without treating a transport flag as a USD path.
///
/// The delivered launch shape is `usdview --headless --webrtc
/// [path/to/stage.usda]`. The asset may appear before or after the transport
/// option.
pub(crate) fn parse_launch_options<I>(arguments: I) -> Result<LaunchOptions, String>
where
    I: IntoIterator<Item = String>,
{
    let mut options = LaunchOptions {
        width: 1920,
        height: 1080,
        fps: 60,
        ..Default::default()
    };
    let mut arguments = arguments.into_iter();
    let mut parse_options = true;

    while let Some(argument) = arguments.next() {
        if parse_options && argument == "--" {
            parse_options = false;
            continue;
        }

        if parse_options && argument == "--headless" {
            options.headless = true;
            continue;
        }

        if parse_options && argument == "--webrtc" {
            options.transport = Some(ViewportTransport::WebRtc);
            continue;
        }

        if parse_options && argument == "--preset" {
            let preset = arguments
                .next()
                .ok_or_else(|| "--preset requires performance|quality|adaptive".to_owned())?;
            match preset.as_str() {
                "performance" => {
                    options.width = 1920;
                    options.height = 1080;
                    options.fps = 120;
                }
                "quality" => {
                    options.width = 2560;
                    options.height = 1440;
                    options.fps = 60;
                }
                "adaptive" => {
                    options.width = 1280;
                    options.height = 720;
                    options.fps = 60;
                }
                other => return Err(format!("unknown preset `{other}`")),
            }
            continue;
        }

        if parse_options && argument == "--codec" {
            let codec = arguments
                .next()
                .ok_or_else(|| "--codec requires h264|h265|av1".to_owned())?;
            options.codec = parse_codec(&codec)?;
            continue;
        }

        if parse_options && argument == "--transport" {
            let transport = arguments
                .next()
                .ok_or_else(|| "--transport requires the delivered value `webrtc`".to_owned())?;
            options.transport = Some(parse_transport(&transport)?);
            continue;
        }

        if parse_options && argument == "--benchmark" {
            options.benchmark = true;
            continue;
        }

        if parse_options && argument == "--benchmark-renderer-matrix" {
            options.benchmark = true;
            options.benchmark_renderer_matrix = true;
            continue;
        }

        if parse_options && argument == "--benchmark-scenario" {
            let sc = arguments
                .next()
                .ok_or_else(|| "--benchmark-scenario requires an identifier like S1".to_owned())?;
            options.benchmark_scenario = Some(sc);
            continue;
        }

        if parse_options && argument == "--benchmark-warmup-frames" {
            let warmup = arguments
                .next()
                .ok_or_else(|| "--benchmark-warmup-frames requires an integer".to_owned())?;
            options.benchmark_warmup_frames = warmup
                .parse::<u64>()
                .map_err(|e| format!("invalid warmup frames: {e}"))?;
            continue;
        }

        if parse_options && argument == "--benchmark-frames" {
            let frames = arguments
                .next()
                .ok_or_else(|| "--benchmark-frames requires an integer".to_owned())?;
            options.benchmark_frames = frames
                .parse::<u64>()
                .map_err(|e| format!("invalid frames count: {e}"))?;
            continue;
        }

        if parse_options && argument == "--benchmark-output" {
            let output = arguments
                .next()
                .ok_or_else(|| "--benchmark-output requires a file path".to_owned())?;
            options.benchmark_output = Some(output);
            continue;
        }

        if parse_options && argument == "--benchmark-label" {
            let label = arguments
                .next()
                .ok_or_else(|| "--benchmark-label requires a label string".to_owned())?;
            options.benchmark_label = label;
            continue;
        }

        if parse_options && argument == "--benchmark-client-ready-file" {
            options.benchmark_client_ready_file =
                Some(arguments.next().ok_or_else(|| {
                    "--benchmark-client-ready-file requires a file path".to_owned()
                })?);
            continue;
        }

        if parse_options && argument == "--benchmark-measurement-start-file" {
            options.benchmark_measurement_start_file = Some(arguments.next().ok_or_else(|| {
                "--benchmark-measurement-start-file requires a file path".to_owned()
            })?);
            continue;
        }

        if parse_options && argument == "--benchmark-measurement-idle-file" {
            options.benchmark_measurement_idle_file = Some(arguments.next().ok_or_else(|| {
                "--benchmark-measurement-idle-file requires a file path".to_owned()
            })?);
            continue;
        }

        if parse_options && argument == "--benchmark-measurement-complete-file" {
            options.benchmark_measurement_complete_file =
                Some(arguments.next().ok_or_else(|| {
                    "--benchmark-measurement-complete-file requires a file path".to_owned()
                })?);
            continue;
        }

        if parse_options {
            if let Some(transport) = argument.strip_prefix("--transport=") {
                options.transport = Some(parse_transport(transport)?);
                continue;
            }
            if let Some(codec) = argument.strip_prefix("--codec=") {
                options.codec = parse_codec(codec)?;
                continue;
            }
            if let Some(sc) = argument.strip_prefix("--benchmark-scenario=") {
                options.benchmark_scenario = Some(sc.to_string());
                continue;
            }
            if let Some(out) = argument.strip_prefix("--benchmark-output=") {
                options.benchmark_output = Some(out.to_string());
                continue;
            }
            if let Some(lbl) = argument.strip_prefix("--benchmark-label=") {
                options.benchmark_label = lbl.to_string();
                continue;
            }
            if let Some(path) = argument.strip_prefix("--benchmark-client-ready-file=") {
                options.benchmark_client_ready_file = Some(path.to_string());
                continue;
            }
            if let Some(path) = argument.strip_prefix("--benchmark-measurement-start-file=") {
                options.benchmark_measurement_start_file = Some(path.to_string());
                continue;
            }
            if let Some(path) = argument.strip_prefix("--benchmark-measurement-idle-file=") {
                options.benchmark_measurement_idle_file = Some(path.to_string());
                continue;
            }
            if let Some(path) = argument.strip_prefix("--benchmark-measurement-complete-file=") {
                options.benchmark_measurement_complete_file = Some(path.to_string());
                continue;
            }
            if argument.starts_with('-') {
                return Err(format!("unrecognized option `{argument}`"));
            }
        }

        if options.asset_argument.replace(argument).is_some() {
            return Err("expected at most one USD asset path".to_owned());
        }
    }

    Ok(options)
}

fn parse_transport(value: &str) -> Result<ViewportTransport, String> {
    match value {
        "webrtc" => Ok(ViewportTransport::WebRtc),
        unsupported => Err(format!(
            "unsupported transport `{unsupported}`; available transport: `webrtc`"
        )),
    }
}

fn parse_codec(value: &str) -> Result<CodecId, String> {
    match value {
        "h264" => Ok(CodecId::H264),
        "h265" => Ok(CodecId::H265),
        "av1" => Ok(CodecId::Av1),
        unsupported => Err(format!(
            "unsupported codec `{unsupported}`; available codecs: `h264`, `h265`, `av1`"
        )),
    }
}

#[cfg(test)]
mod tests {
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
    fn renderer_matrix_benchmark_flag_selects_the_release_matrix_runner() {
        let options = parse_launch_options(vec!["--benchmark-renderer-matrix".to_owned()]).unwrap();

        assert!(options.benchmark);
        assert!(options.benchmark_renderer_matrix);
    }
}
