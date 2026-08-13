//! Render-server transport selection and adapters for the viewport process.

use viewport_protocol::CodecId;

pub(crate) mod frame_capture;
pub(crate) mod webrtc;

pub(crate) use frame_capture::{FrameCapturePlugin, FrameData};

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

        if parse_options {
            if let Some(transport) = argument.strip_prefix("--transport=") {
                options.transport = Some(parse_transport(transport)?);
                continue;
            }
            if let Some(codec) = argument.strip_prefix("--codec=") {
                options.codec = parse_codec(codec)?;
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
}
