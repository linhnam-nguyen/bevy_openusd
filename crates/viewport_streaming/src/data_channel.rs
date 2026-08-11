//! WebRTC DataChannel construction and lifecycle diagnostics.
//!
//! The server creates both application channels before generating its SDP
//! offer. The reliable control channel also owns the application handshake;
//! semantic viewport commands remain queued for the Phase 3 bridge.

use anyhow::{Context, Result};
use gstreamer::prelude::*;
use gstreamer_webrtc::WebRTCDataChannel;
use log::{debug, error, info, warn};
use std::sync::{Arc, Mutex};
use viewport_protocol::{
    ClientCommand, CommandFamily, HandshakeEvent, HandshakeRejectionReason, PROTOCOL_VERSION,
    ProtocolValidationError, ServerCapabilities, ServerEvent, ServerEventEnvelope, SessionEvent,
    SessionCommand, SessionId, ViewportCommandEnvelope, ViewportReadModel,
    decode_client_json_line, encode_server_json_line,
};

use crate::channel_backpressure::CONTROL_LOW_WATER_MARK;
use crate::RenderServerInterface;

pub const CONTROL_CHANNEL_LABEL: &str = "viewport-control";
pub const INPUT_CHANNEL_LABEL: &str = "viewport-input";
pub const CONTROL_CHANNEL_PROTOCOL: &str = "usd-hub.viewport.v1";
pub const INPUT_CHANNEL_PROTOCOL: &str = "usd-hub.viewport-input.v1";

/// Application state shared by the two callbacks attached to one control
/// DataChannel. It deliberately knows only the transport-neutral protocol.
#[derive(Clone)]
pub(crate) struct ApplicationSession {
    state: Arc<Mutex<ApplicationSessionState>>,
}

struct ApplicationSessionState {
    session_id: SessionId,
    server_capabilities: ServerCapabilities,
    initial_snapshot: ViewportReadModel,
    interface: RenderServerInterface,
    client_sequence: u64,
    server_sequence: u64,
    handshaken: bool,
}

impl ApplicationSession {
    pub(crate) fn new(
        session_id: SessionId,
        initial_snapshot: ViewportReadModel,
        interface: RenderServerInterface,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(ApplicationSessionState {
                session_id,
                server_capabilities: ServerCapabilities::default(),
                initial_snapshot,
                interface,
                client_sequence: 0,
                server_sequence: 1,
                handshaken: false,
            })),
        }
    }

    fn handle_control_message(&self, channel: &WebRTCDataChannel, text: &str) {
        let envelope = match decode_client_json_line(text) {
            Ok(envelope) => envelope,
            Err(error) => {
                debug!("[viewport-data-channel] ignoring non-application control payload: {error}");
                return;
            }
        };

        let Ok(mut state) = self.state.lock() else {
            error!("[viewport-data-channel] application session state is poisoned");
            return;
        };

        if let Err(error) = envelope.validate() {
            if !state.handshaken {
                send_handshake_rejection(channel, &mut state, rejection_for(error));
            } else {
                warn!("[viewport-data-channel] rejected invalid command envelope: {error}");
            }
            return;
        }

        if envelope.sequence <= state.client_sequence {
            warn!(
                "[viewport-data-channel] ignoring stale client sequence {} (last {})",
                envelope.sequence, state.client_sequence
            );
            return;
        }

        if !state.handshaken {
            let ClientCommand::Handshake(hello) = envelope.command else {
                send_handshake_rejection(
                    channel,
                    &mut state,
                    HandshakeRejectionReason::InvalidClientIdentity,
                );
                return;
            };

            if envelope.session_id.is_some() {
                send_handshake_rejection(
                    channel,
                    &mut state,
                    HandshakeRejectionReason::InvalidClientIdentity,
                );
                return;
            }

            if let Err(error) = hello.validate() {
                send_handshake_rejection(
                    channel,
                    &mut state,
                    rejection_for(error),
                );
                return;
            }
            if !hello
                .capabilities
                .protocol_versions
                .contains(&PROTOCOL_VERSION)
            {
                send_handshake_rejection(
                    channel,
                    &mut state,
                    HandshakeRejectionReason::UnsupportedProtocolVersion,
                );
                return;
            }
            if !hello
                .capabilities
                .command_families
                .contains(&CommandFamily::Session)
            {
                send_handshake_rejection(
                    channel,
                    &mut state,
                    HandshakeRejectionReason::UnsupportedCapabilities,
                );
                return;
            }

            state.client_sequence = envelope.sequence;
            state.handshaken = true;
            let session_id = state.session_id.clone();
            let capabilities = state.server_capabilities.clone();
            let snapshot = state
                .interface
                .take_latest_snapshot(state.initial_snapshot.clone());
            send_server_event(
                channel,
                &mut state,
                ServerEvent::Handshake(HandshakeEvent::ServerHello(
                    viewport_protocol::ServerHello::new(
                        session_id.clone(),
                        hello.requested_role,
                        capabilities,
                    ),
                )),
            );
            send_server_event(
                channel,
                &mut state,
                ServerEvent::Session(SessionEvent::Ready {
                    snapshot_required: true,
                }),
            );
            send_server_event(
                channel,
                &mut state,
                ServerEvent::Session(SessionEvent::Snapshot { state: snapshot }),
            );
            return;
        }

        if envelope.session_id.as_ref() != Some(&state.session_id) {
            warn!(
                "[viewport-data-channel] rejected command for a different session: {:?}",
                envelope.session_id
            );
            return;
        }

        state.client_sequence = envelope.sequence;
        match envelope.command {
            ClientCommand::Viewport(command) => {
                let viewport_command = ViewportCommandEnvelope {
                    protocol_version: envelope.protocol_version,
                    request_id: envelope.request_id,
                    command,
                };
                if let Err(error) = state.interface.submit_viewport_command(viewport_command) {
                    warn!("[viewport-data-channel] rejected viewport command: {error:?}");
                }
            }
            ClientCommand::Session(SessionCommand::RequestSnapshot) => {
                let snapshot = state
                    .interface
                    .take_latest_snapshot(state.initial_snapshot.clone());
                send_server_event(
                    channel,
                    &mut state,
                    ServerEvent::Session(SessionEvent::Snapshot { state: snapshot }),
                );
            }
            command => {
                debug!(
                    "[viewport-data-channel] accepted non-viewport command for a later phase: {command:?}"
                );
            }
        }
    }

    pub(crate) fn flush_authoritative_events(&self, channel: &WebRTCDataChannel) {
        let Ok(mut state) = self.state.lock() else {
            error!("[viewport-data-channel] application session state is poisoned");
            return;
        };
        if !state.handshaken {
            return;
        }

        let interface = state.interface.clone();
        while let Some(event) = interface.pop_viewport_event() {
            let server_event = ServerEvent::Viewport(event.event);
            let envelope = match event.request_id {
                Some(request_id) => ServerEventEnvelope::for_request(
                    state.session_id.clone(),
                    state.server_sequence,
                    request_id,
                    server_event,
                ),
                None => ServerEventEnvelope::new(
                    state.session_id.clone(),
                    state.server_sequence,
                    server_event,
                ),
            };
            state.server_sequence = state.server_sequence.saturating_add(1);
            match encode_server_json_line(&envelope)
                .map_err(|error| error.to_string())
                .and_then(|json| {
                    channel
                        .send_string_full(Some(&json))
                        .map_err(|error| error.to_string())
                }) {
                Ok(()) => {}
                Err(error) => {
                    warn!("[viewport-data-channel] authoritative event send failed: {error}");
                    break;
                }
            }
        }
    }
}

fn rejection_for(error: ProtocolValidationError) -> HandshakeRejectionReason {
    match error {
        ProtocolValidationError::UnsupportedProtocolVersion { .. } => {
            HandshakeRejectionReason::UnsupportedProtocolVersion
        }
        ProtocolValidationError::EmptyField { .. } => {
            HandshakeRejectionReason::InvalidClientIdentity
        }
        _ => HandshakeRejectionReason::UnsupportedCapabilities,
    }
}

fn send_handshake_rejection(
    channel: &WebRTCDataChannel,
    state: &mut ApplicationSessionState,
    reason: HandshakeRejectionReason,
) {
    send_server_event(
        channel,
        state,
        ServerEvent::Handshake(HandshakeEvent::Rejected { reason }),
    );
}

fn send_server_event(
    channel: &WebRTCDataChannel,
    state: &mut ApplicationSessionState,
    event: ServerEvent,
) {
    let envelope = ServerEventEnvelope::new(state.session_id.clone(), state.server_sequence, event);
    state.server_sequence = state.server_sequence.saturating_add(1);
    match encode_server_json_line(&envelope)
        .map_err(|error| error.to_string())
        .and_then(|json| channel.send_string_full(Some(&json)).map_err(|error| error.to_string()))
    {
        Ok(()) => {}
        Err(error) => warn!("[viewport-data-channel] application event send failed: {error}"),
    }
}

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

        if !control {
            debug!(
                "[viewport-data-channel] ignoring provisional {} payload: {}",
                message_label, message
            );
            return;
        }
        application_for_message.handle_control_message(channel, message);
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

}
