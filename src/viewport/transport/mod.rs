//! Native transport selection and adapters for the viewport process.
//!
//! The default `usdview` invocation remains a standalone Frost viewer. Passing
//! `--stdio` or `--transport stdio` additionally exposes the UI-neutral
//! `viewport_protocol` contract over JSON Lines on standard input/output.

mod stdio;

pub(crate) use stdio::StdioTransportPlugin;

/// A process boundary enabled for this viewport launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewportTransport {
    /// JSON Lines commands on stdin and events on stdout.
    Stdio,
}

/// Command-line configuration that is independent of Bevy startup.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct LaunchOptions {
    pub(crate) asset_argument: Option<String>,
    pub(crate) transport: Option<ViewportTransport>,
}

/// Parses `usdview` arguments without treating a transport flag as a USD path.
///
/// The ordinary launch shape is still `usdview [path/to/stage.usda]`. For a
/// native host process, either `usdview --stdio` or
/// `usdview --transport stdio` enables the JSON Lines adapter. The asset may
/// appear before or after the transport option.
pub(crate) fn parse_launch_options<I>(arguments: I) -> Result<LaunchOptions, String>
where
    I: IntoIterator<Item = String>,
{
    let mut options = LaunchOptions::default();
    let mut arguments = arguments.into_iter();
    let mut parse_options = true;

    while let Some(argument) = arguments.next() {
        if parse_options && argument == "--" {
            parse_options = false;
            continue;
        }

        if parse_options && argument == "--stdio" {
            options.transport = Some(ViewportTransport::Stdio);
            continue;
        }

        if parse_options && argument == "--transport" {
            let transport = arguments
                .next()
                .ok_or_else(|| "--transport requires a value such as `stdio`".to_owned())?;
            options.transport = Some(parse_transport(&transport)?);
            continue;
        }

        if parse_options {
            if let Some(transport) = argument.strip_prefix("--transport=") {
                options.transport = Some(parse_transport(transport)?);
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
        "stdio" => Ok(ViewportTransport::Stdio),
        unsupported => Err(format!(
            "unsupported transport `{unsupported}`; only `stdio` is available in this build"
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
            }
        );
    }

    #[test]
    fn stdio_flags_do_not_become_asset_paths() {
        for arguments in [
            vec!["--stdio".to_owned()],
            vec!["--transport".to_owned(), "stdio".to_owned()],
            vec!["--transport=stdio".to_owned()],
        ] {
            assert_eq!(
                parse_launch_options(arguments).unwrap(),
                LaunchOptions {
                    asset_argument: None,
                    transport: Some(ViewportTransport::Stdio),
                }
            );
        }
    }

    #[test]
    fn transport_and_asset_can_be_supplied_in_either_order() {
        for arguments in [
            vec!["--stdio".to_owned(), "fixtures/robot.usda".to_owned()],
            vec![
                "fixtures/robot.usda".to_owned(),
                "--transport".to_owned(),
                "stdio".to_owned(),
            ],
        ] {
            assert_eq!(
                parse_launch_options(arguments).unwrap(),
                LaunchOptions {
                    asset_argument: Some("fixtures/robot.usda".to_owned()),
                    transport: Some(ViewportTransport::Stdio),
                }
            );
        }
    }
}
