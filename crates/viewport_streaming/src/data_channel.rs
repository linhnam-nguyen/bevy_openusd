//! WebRTC DataChannel construction and lifecycle diagnostics.
//!
//! The server creates both application channels before generating its SDP
//! offer. Product commands are intentionally not routed here yet; Phase 1
//! only proves channel ownership, lifecycle callbacks, and a diagnostic
//! ping/pong on the reliable control channel.

use anyhow::{Context, Result};
use gstreamer::prelude::*;
use gstreamer_webrtc::WebRTCDataChannel;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};

use crate::channel_backpressure::CONTROL_LOW_WATER_MARK;

pub const CONTROL_CHANNEL_LABEL: &str = "viewport-control";
pub const INPUT_CHANNEL_LABEL: &str = "viewport-input";
pub const CONTROL_CHANNEL_PROTOCOL: &str = "usd-hub.viewport.v1";
pub const INPUT_CHANNEL_PROTOCOL: &str = "usd-hub.viewport-input.v1";

/// The WebRTC channel configuration that is sent to webrtcbin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelOptions {
    pub label: &'static str,
    pub ordered: bool,
    pub max_retransmits: Option<i32>,
    pub protocol: &'static str,
}

impl ChannelOptions {
    pub const fn control() -> Self {
        Self {
            label: CONTROL_CHANNEL_LABEL,
            ordered: true,
            max_retransmits: None,
            protocol: CONTROL_CHANNEL_PROTOCOL,
        }
    }

    pub const fn input() -> Self {
        Self {
            label: INPUT_CHANNEL_LABEL,
            ordered: false,
            max_retransmits: Some(0),
            protocol: INPUT_CHANNEL_PROTOCOL,
        }
    }

    pub fn to_gstreamer_options(self) -> gstreamer::Structure {
        let mut builder = gstreamer::Structure::builder("viewport-data-channel")
            .field("ordered", self.ordered)
            .field("protocol", self.protocol);

        if let Some(max_retransmits) = self.max_retransmits {
            builder = builder.field("max-retransmits", max_retransmits);
        }

        builder.build()
    }
}

/// The two server-created channels owned by one streaming session.
#[derive(Clone)]
pub struct DataChannelSet {
    control: WebRTCDataChannel,
    input: WebRTCDataChannel,
}

impl DataChannelSet {
    /// Installs prepare-data-channel before creating either local channel.
    ///
    /// GStreamer documents this callback as the point where consumers can
    /// attach handlers before channel state or data notifications are emitted.
    pub fn create(webrtc: &gstreamer::Element) -> Result<Self> {
        install_prepare_callback(webrtc);

        let control = create_channel(webrtc, ChannelOptions::control())?;
        let input = create_channel(webrtc, ChannelOptions::input())?;

        Ok(Self { control, input })
    }

    pub fn control(&self) -> &WebRTCDataChannel {
        &self.control
    }

    pub fn input(&self) -> &WebRTCDataChannel {
        &self.input
    }

    pub fn close(&self) {
        self.control.close();
        self.input.close();
    }
}

fn install_prepare_callback(webrtc: &gstreamer::Element) {
    webrtc.connect("prepare-data-channel", false, move |values| {
        let Some(channel) = values
            .get(1)
            .and_then(|value| value.get::<WebRTCDataChannel>().ok())
        else {
            warn!("[viewport-data-channel] prepare-data-channel had no channel");
            return None;
        };

        let is_local = values
            .get(2)
            .and_then(|value| value.get::<bool>().ok())
            .unwrap_or(false);
        attach_channel_callbacks(&channel, is_local);
        None
    });
}

fn create_channel(
    webrtc: &gstreamer::Element,
    options: ChannelOptions,
) -> Result<WebRTCDataChannel> {
    let gst_options = options.to_gstreamer_options();
    let channel = webrtc
        .emit_by_name::<Option<WebRTCDataChannel>>(
            "create-data-channel",
            &[&options.label, &gst_options],
        )
        .context("webrtcbin could not create a DataChannel; is usrsctp available?")?;

    let actual_label = channel.label().map(|label| label.to_string());
    if actual_label.as_deref() != Some(options.label) {
        anyhow::bail!(
            "webrtcbin returned DataChannel label {:?}, expected {:?}",
            actual_label,
            options.label
        );
    }

    if channel.is_ordered() != options.ordered {
        anyhow::bail!(
            "DataChannel {} ordered={} but expected {}",
            options.label,
            channel.is_ordered(),
            options.ordered
        );
    }

    let expected_retransmits = options.max_retransmits.unwrap_or(-1);
    if channel.max_retransmits() != expected_retransmits {
        warn!(
            "[viewport-data-channel] {} max-retransmits reported as {}, expected {}",
            options.label,
            channel.max_retransmits(),
            expected_retransmits
        );
    }

    Ok(channel)
}

fn attach_channel_callbacks(channel: &WebRTCDataChannel, is_local: bool) {
    let label = channel
        .label()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<unnamed>".to_owned());
    let control = label == CONTROL_CHANNEL_LABEL;

    if control {
        channel.set_buffered_amount_low_threshold(CONTROL_LOW_WATER_MARK);
        let low_label = label.clone();
        channel.connect_on_buffered_amount_low(move |channel| {
            debug!(
                "[viewport-data-channel] {} reached low-water mark with {} buffered bytes",
                low_label,
                channel.buffered_amount()
            );
        });
    }

    let open_label = label.clone();
    channel.connect_on_open(move |channel| {
        info!(
            "[viewport-data-channel] {} opened (id={}, local={is_local})",
            open_label,
            channel.id()
        );

        if control {
            let message = DiagnosticControlMessage::Ping {
                nonce: format!("server-open-{}", channel.id()),
            };
            send_diagnostic(channel, &message);
        }
    });

    let message_label = label.clone();
    channel.connect_on_message_string(move |channel, message| {
        let Some(message) = message else {
            warn!(
                "[viewport-data-channel] {} received an empty message",
                message_label
            );
            return;
        };

        if !control {
            debug!(
                "[viewport-data-channel] ignoring provisional {} payload: {}",
                message_label, message
            );
            return;
        }

        match serde_json::from_str::<DiagnosticControlMessage>(message) {
            Ok(DiagnosticControlMessage::Ping { nonce }) => {
                send_diagnostic(channel, &DiagnosticControlMessage::Pong { nonce });
            }
            Ok(DiagnosticControlMessage::Pong { nonce }) => {
                info!("[viewport-data-channel] control diagnostic pong received: {nonce}");
            }
            Err(error) => {
                debug!(
                    "[viewport-data-channel] control payload is not a Phase 1 diagnostic: {error}"
                );
            }
        }
    });

    let close_label = label.clone();
    channel.connect_on_close(move |_| {
        info!("[viewport-data-channel] {} closed", close_label);
    });

    let error_label = label;
    channel.connect_on_error(move |_, error| {
        error!("[viewport-data-channel] {} error: {}", error_label, error);
    });
}

fn send_diagnostic(channel: &WebRTCDataChannel, message: &DiagnosticControlMessage) {
    match serde_json::to_string(message)
        .map_err(|error| error.to_string())
        .and_then(|json| {
            channel
                .send_string_full(Some(&json))
                .map_err(|error| error.to_string())
        }) {
        Ok(()) => {}
        Err(error) => warn!("[viewport-data-channel] diagnostic send failed: {error}"),
    }
}

/// The only application payload interpreted during Phase 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DiagnosticControlMessage {
    Ping { nonce: String },
    Pong { nonce: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_channel_is_ordered_and_reliable_by_default() {
        gstreamer::init().unwrap();
        let options = ChannelOptions::control();
        let structure = options.to_gstreamer_options();

        assert!(structure.get::<bool>("ordered").unwrap());
        assert_eq!(
            structure.get::<String>("protocol").unwrap(),
            CONTROL_CHANNEL_PROTOCOL
        );
        assert!(structure.get::<i32>("max-retransmits").is_err());
    }

    #[test]
    fn input_channel_is_unordered_and_drop_eligible() {
        gstreamer::init().unwrap();
        let options = ChannelOptions::input();
        let structure = options.to_gstreamer_options();

        assert!(!structure.get::<bool>("ordered").unwrap());
        assert_eq!(structure.get::<i32>("max-retransmits").unwrap(), 0);
        assert_eq!(
            structure.get::<String>("protocol").unwrap(),
            INPUT_CHANNEL_PROTOCOL
        );
    }

    #[test]
    fn diagnostic_messages_round_trip() {
        for message in [
            DiagnosticControlMessage::Ping {
                nonce: "client-1".to_owned(),
            },
            DiagnosticControlMessage::Pong {
                nonce: "server-1".to_owned(),
            },
        ] {
            let json = serde_json::to_string(&message).unwrap();
            assert_eq!(
                serde_json::from_str::<DiagnosticControlMessage>(&json).unwrap(),
                message
            );
        }
    }
}
