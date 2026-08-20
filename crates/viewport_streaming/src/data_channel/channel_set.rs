use anyhow::{Context, Result};
use gstreamer::prelude::*;
use gstreamer_webrtc::WebRTCDataChannel;
use log::{debug, error, info, warn};

use super::constants::{
    CONTROL_CHANNEL_LABEL, CONTROL_CHANNEL_LOW_WATER_MARK_BYTES, CONTROL_CHANNEL_PROTOCOL,
    INPUT_CHANNEL_LABEL, INPUT_CHANNEL_PROTOCOL,
};
use super::session::ApplicationSession;

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
    pub(crate) fn create(
        webrtc: &gstreamer::Element,
        application: ApplicationSession,
    ) -> Result<Self> {
        install_prepare_callback(webrtc, application);

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

fn install_prepare_callback(webrtc: &gstreamer::Element, application: ApplicationSession) {
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
        attach_channel_callbacks(&channel, is_local, application.clone());
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

fn attach_channel_callbacks(
    channel: &WebRTCDataChannel,
    is_local: bool,
    application: ApplicationSession,
) {
    let label = channel
        .label()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<unnamed>".to_owned());
    let control = label == CONTROL_CHANNEL_LABEL;

    if control {
        channel.set_buffered_amount_low_threshold(CONTROL_CHANNEL_LOW_WATER_MARK_BYTES);
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
    });

    let application_for_message = application.clone();
    let message_label = label.clone();
    channel.connect_on_message_string(move |channel, message| {
        let Some(message) = message else {
            warn!(
                "[viewport-data-channel] {} received an empty message",
                message_label
            );
            return;
        };

        if control {
            application_for_message.handle_control_message(channel, message);
        } else {
            application_for_message.handle_input_message(message);
        }
    });

    let close_label = label.clone();
    let application_for_close = application.clone();
    channel.connect_on_close(move |_| {
        application_for_close.clear_remote_input();
        info!("[viewport-data-channel] {} closed", close_label);
    });

    let error_label = label;
    channel.connect_on_error(move |_, error| {
        error!("[viewport-data-channel] {} error: {}", error_label, error);
    });
}
