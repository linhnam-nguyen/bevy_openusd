mod commands;
mod handshake;
mod send;

pub(crate) use send::{encoded_size, flush_pending_server_events, next_server_envelope};

use gstreamer_webrtc::WebRTCDataChannel;
use log::{debug, error, warn};
use viewport_protocol::{InputCommand, SessionRole, decode_client_json_line};

use crate::data_channel::session::{ApplicationSession, remember_request_id};
use commands::handle_authenticated_command;
use handshake::handle_handshake;
use send::{rejection_for, send_command_rejection, send_handshake_rejection};

impl ApplicationSession {
    pub(super) fn handle_control_message(&self, channel: &WebRTCDataChannel, text: &str) {
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
            handle_handshake(channel, &mut state, envelope);
            return;
        }

        if envelope.session_id.as_ref() != Some(&state.session_id) {
            warn!(
                "[viewport-data-channel] rejected command for a different session: {:?}",
                envelope.session_id
            );
            return;
        }

        if envelope.sequence != state.client_sequence.saturating_add(1) {
            state.client_sequence = envelope.sequence;
            send_command_rejection(
                channel,
                &mut state,
                envelope.request_id,
                "client command sequence was not contiguous".to_owned(),
            );
            return;
        }

        state.client_sequence = envelope.sequence;
        if !remember_request_id(&mut state, envelope.request_id.clone()) {
            send_command_rejection(
                channel,
                &mut state,
                envelope.request_id,
                "duplicate request ID".to_owned(),
            );
            return;
        }

        handle_authenticated_command(channel, &mut state, envelope);
    }

    pub(super) fn handle_input_message(&self, text: &str) {
        let command = match serde_json::from_str::<InputCommand>(text) {
            Ok(command) => command,
            Err(error) => {
                debug!("[viewport-data-channel] ignoring invalid motion payload: {error}");
                return;
            }
        };

        if !matches!(command, InputCommand::PointerMotion(_)) {
            warn!("[viewport-data-channel] unordered input channel received non-motion input");
            return;
        }

        let Ok(state) = self.state.lock() else {
            error!("[viewport-data-channel] application session state is poisoned");
            return;
        };
        if !state.handshaken || state.role != Some(SessionRole::Controller) {
            return;
        }
        if let Err(error) = state.interface.submit_input(command) {
            debug!("[viewport-data-channel] dropped motion payload: {error:?}");
        }
    }
}
