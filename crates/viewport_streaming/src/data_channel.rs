//! WebRTC DataChannel construction and lifecycle diagnostics.
//!
//! The server creates both application channels before generating its SDP
//! offer. The reliable control channel also owns the application handshake;
//! semantic viewport commands remain queued for the Phase 3 bridge.

use anyhow::{Context, Result};
use gstreamer::prelude::*;
use gstreamer_webrtc::WebRTCDataChannel;
use log::{debug, error, info, warn};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use viewport_protocol::{
    ActiveStreamConfiguration, ClientCommand, CommandFamily, HandshakeEvent,
    HandshakeRejectionReason, InputCommand, PROTOCOL_VERSION, ProtocolValidationError,
    ServerCapabilities, ServerEvent, ServerEventEnvelope, SessionCommand, SessionEvent, SessionId,
    StreamCommand, StreamEvent, ViewportCommandEnvelope, ViewportEvent, ViewportReadModel,
    decode_client_json_line, encode_server_json_line,
};

use crate::RenderServerInterface;
use crate::channel_backpressure::CONTROL_LOW_WATER_MARK;

pub const CONTROL_CHANNEL_LABEL: &str = "viewport-control";
pub const INPUT_CHANNEL_LABEL: &str = "viewport-input";
pub const CONTROL_CHANNEL_PROTOCOL: &str = "usd-hub.viewport.v1";
pub const INPUT_CHANNEL_PROTOCOL: &str = "usd-hub.viewport-input.v1";

// Browser DataChannels commonly reject application messages around 16 KiB.
// Keep a safety margin for the JSON envelope and browser/runtime variation.
const MAX_APPLICATION_MESSAGE_BYTES: usize = 12 * 1024;
const INITIAL_SNAPSHOT_CHUNK_PRIMS: usize = 128;

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
    /// The session manager polls this replaceable slot at its regular event
    /// cadence. Keeping only the newest accepted request prevents a layout
    /// drag from queuing obsolete encoder transactions behind the current one.
    pending_stream_configuration: Option<viewport_protocol::ViewportMetrics>,
    latest_stream_generation: u64,
    client_sequence: u64,
    server_sequence: u64,
    handshaken: bool,
    recent_request_ids: VecDeque<String>,
    pending_server_events: VecDeque<ServerEventEnvelope>,
}

impl ApplicationSession {
    #[cfg(test)]
    pub(crate) fn new(
        session_id: SessionId,
        initial_snapshot: ViewportReadModel,
        interface: RenderServerInterface,
    ) -> Self {
        Self::new_with_capabilities(
            session_id,
            initial_snapshot,
            interface,
            ServerCapabilities::default(),
        )
    }

    pub(crate) fn new_with_capabilities(
        session_id: SessionId,
        initial_snapshot: ViewportReadModel,
        interface: RenderServerInterface,
        server_capabilities: ServerCapabilities,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(ApplicationSessionState {
                session_id,
                server_capabilities,
                initial_snapshot,
                interface,
                pending_stream_configuration: None,
                latest_stream_generation: 0,
                client_sequence: 0,
                server_sequence: 1,
                handshaken: false,
                recent_request_ids: VecDeque::new(),
                pending_server_events: VecDeque::new(),
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

        let request_id = envelope.request_id.clone();

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
                send_handshake_rejection(channel, &mut state, rejection_for(error));
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

            let requested_metrics = state
                .server_capabilities
                .stream_limits
                .normalize(&hello.initial_viewport);
            let initial_metrics = match state
                .interface
                .submit_stream_configuration(requested_metrics)
            {
                Ok(metrics) => metrics,
                Err(error) => {
                    warn!(
                        "[viewport-data-channel] initial stream configuration rejected: {error:?}"
                    );
                    send_handshake_rejection(
                        channel,
                        &mut state,
                        HandshakeRejectionReason::UnsupportedCapabilities,
                    );
                    return;
                }
            };
            state.pending_stream_configuration = Some(initial_metrics.clone());
            state.latest_stream_generation = initial_metrics.generation;

            state.client_sequence = envelope.sequence;
            remember_request_id(&mut state, request_id);
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
                ServerEvent::Stream(StreamEvent::ConfigurationAccepted {
                    metrics: initial_metrics,
                }),
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

        if envelope.sequence != state.client_sequence.saturating_add(1) {
            state.client_sequence = envelope.sequence;
            send_command_rejection(
                channel,
                &mut state,
                request_id,
                "client command sequence was not contiguous".to_owned(),
            );
            return;
        }

        state.client_sequence = envelope.sequence;
        if !remember_request_id(&mut state, request_id.clone()) {
            send_command_rejection(
                channel,
                &mut state,
                request_id,
                "duplicate request ID".to_owned(),
            );
            return;
        }

        match envelope.command {
            ClientCommand::Viewport(command) => {
                let viewport_command = ViewportCommandEnvelope {
                    protocol_version: envelope.protocol_version,
                    request_id: request_id.clone(),
                    command,
                };
                if let Err(error) = state.interface.submit_viewport_command(viewport_command) {
                    send_command_rejection(
                        channel,
                        &mut state,
                        request_id,
                        format!("viewport command rejected: {error:?}"),
                    );
                }
            }
            ClientCommand::Input(command) => {
                if let Err(error) = state.interface.submit_input(command) {
                    send_command_rejection(
                        channel,
                        &mut state,
                        request_id,
                        format!("input command rejected: {error:?}"),
                    );
                }
            }
            ClientCommand::Session(SessionCommand::RequestSnapshot) => {
                let snapshot = state
                    .interface
                    .take_latest_snapshot(state.initial_snapshot.clone());
                send_server_event_for_request(
                    channel,
                    &mut state,
                    request_id,
                    ServerEvent::Session(SessionEvent::Snapshot { state: snapshot }),
                );
            }
            ClientCommand::Session(SessionCommand::Ping { nonce }) => {
                send_server_event_for_request(
                    channel,
                    &mut state,
                    request_id,
                    ServerEvent::Session(SessionEvent::Pong { nonce }),
                );
            }
            ClientCommand::Stream(StreamCommand::ConfigureViewport { metrics }) => {
                let requested_metrics = state.server_capabilities.stream_limits.normalize(&metrics);
                if requested_metrics.generation <= state.latest_stream_generation {
                    let latest_generation = state.latest_stream_generation;
                    send_stream_configuration_rejection(
                        channel,
                        &mut state,
                        request_id,
                        format!(
                            "stream generation {} is not newer than active generation {}",
                            requested_metrics.generation, latest_generation
                        ),
                    );
                    return;
                }
                let metrics = match state
                    .interface
                    .submit_stream_configuration(requested_metrics)
                {
                    Ok(metrics) => metrics,
                    Err(error) => {
                        send_stream_configuration_rejection(
                            channel,
                            &mut state,
                            request_id,
                            format!("stream configuration rejected: {error:?}"),
                        );
                        return;
                    }
                };
                state.pending_stream_configuration = Some(metrics.clone());
                state.latest_stream_generation = metrics.generation;
                send_server_event_for_request(
                    channel,
                    &mut state,
                    request_id,
                    ServerEvent::Stream(StreamEvent::ConfigurationAccepted { metrics }),
                );
            }
            command => {
                send_command_rejection(
                    channel,
                    &mut state,
                    request_id,
                    format!("command family is not available in this phase: {command:?}"),
                );
            }
        }
    }

    fn handle_input_message(&self, text: &str) {
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
        if !state.handshaken {
            return;
        }
        if let Err(error) = state.interface.submit_input(command) {
            debug!("[viewport-data-channel] dropped motion payload: {error:?}");
        }
    }

    pub(crate) fn clear_remote_input(&self) {
        if let Ok(state) = self.state.lock() {
            state.interface.clear_remote_input();
        }
    }

    /// Returns the newest accepted resize for the encoder coordinator. This
    /// has no side effects on the Bevy-side resize inbox, which is owned by
    /// `RenderServerInterface` and applied on the ECS main thread.
    pub(crate) fn take_stream_configuration(&self) -> Option<viewport_protocol::ViewportMetrics> {
        self.state.lock().ok()?.pending_stream_configuration.take()
    }

    pub(crate) fn queue_configuration_applied(&self, configuration: ActiveStreamConfiguration) {
        let Ok(mut state) = self.state.lock() else {
            error!("[viewport-data-channel] application session state is poisoned");
            return;
        };
        if state.handshaken {
            queue_server_event_for_request(
                &mut state,
                None,
                ServerEvent::Stream(StreamEvent::ConfigurationApplied { configuration }),
            );
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
        flush_pending_server_events(channel, &mut state);
        while state.pending_server_events.is_empty() {
            let Some(event) = interface.pop_viewport_event() else {
                break;
            };
            queue_server_event_for_request(
                &mut state,
                event.request_id,
                ServerEvent::Viewport(event.event),
            );
            flush_pending_server_events(channel, &mut state);
            if !state.pending_server_events.is_empty() {
                break;
            }
        }
    }
}

const MAX_RECENT_REQUEST_IDS: usize = 256;

fn remember_request_id(state: &mut ApplicationSessionState, request_id: String) -> bool {
    if state
        .recent_request_ids
        .iter()
        .any(|seen| seen == &request_id)
    {
        return false;
    }
    if state.recent_request_ids.len() >= MAX_RECENT_REQUEST_IDS {
        state.recent_request_ids.pop_front();
    }
    state.recent_request_ids.push_back(request_id);
    true
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
    send_server_event_with_request(channel, state, None, event);
}

fn send_server_event_for_request(
    channel: &WebRTCDataChannel,
    state: &mut ApplicationSessionState,
    request_id: String,
    event: ServerEvent,
) {
    send_server_event_with_request(channel, state, Some(request_id), event);
}

fn send_command_rejection(
    channel: &WebRTCDataChannel,
    state: &mut ApplicationSessionState,
    request_id: String,
    reason: String,
) {
    send_server_event_for_request(
        channel,
        state,
        request_id.clone(),
        ServerEvent::Viewport(ViewportEvent::CommandRejected { request_id, reason }),
    );
}

fn send_stream_configuration_rejection(
    channel: &WebRTCDataChannel,
    state: &mut ApplicationSessionState,
    request_id: String,
    reason: String,
) {
    send_server_event_for_request(
        channel,
        state,
        request_id,
        ServerEvent::Stream(StreamEvent::ConfigurationRejected { reason }),
    );
}

fn send_server_event_with_request(
    channel: &WebRTCDataChannel,
    state: &mut ApplicationSessionState,
    request_id: Option<String>,
    event: ServerEvent,
) {
    queue_server_event_for_request(state, request_id, event);
    flush_pending_server_events(channel, state);
}

fn queue_server_event_for_request(
    state: &mut ApplicationSessionState,
    request_id: Option<String>,
    event: ServerEvent,
) {
    match event {
        ServerEvent::Session(SessionEvent::Snapshot { state: snapshot }) => {
            queue_snapshot(state, request_id, snapshot, true);
        }
        ServerEvent::Viewport(ViewportEvent::Snapshot { state: snapshot }) => {
            queue_snapshot(state, request_id, snapshot, false);
        }
        ServerEvent::Viewport(ViewportEvent::SearchResults {
            query,
            offset,
            total,
            matches,
            has_more,
        }) => {
            queue_search_results(state, request_id, query, offset, total, matches, has_more);
        }
        ServerEvent::Viewport(ViewportEvent::SceneChildren { page }) => {
            queue_scene_children_page(state, request_id, page);
        }
        event => {
            if !queue_bounded_event(state, request_id.as_deref(), event) {
                warn!(
                    "[viewport-data-channel] dropping oversized application event instead of blocking the queue"
                );
            }
        }
    }
}

fn queue_search_results(
    state: &mut ApplicationSessionState,
    request_id: Option<String>,
    query: String,
    offset: u32,
    total: u32,
    matches: Vec<viewport_protocol::SceneSearchMatch>,
    has_more: bool,
) {
    let event = ServerEvent::Viewport(ViewportEvent::SearchResults {
        query: query.clone(),
        offset,
        total,
        matches: matches.clone(),
        has_more,
    });
    if queue_bounded_event(state, request_id.as_deref(), event) {
        return;
    }

    // A deep path can make one page larger than the browser's DataChannel
    // limit. Split the page while retaining the same request ID and absolute
    // offsets so the frontend can append it deterministically.
    if matches.len() > 1 {
        let split = matches.len() / 2;
        let mut tail = matches;
        let head = tail.split_off(split);
        queue_search_results(
            state,
            request_id.clone(),
            query.clone(),
            offset,
            total,
            tail,
            true,
        );
        queue_search_results(
            state,
            request_id,
            query,
            offset.saturating_add(split as u32),
            total,
            head,
            has_more,
        );
        return;
    }

    // Keep a single result selectable even if its ancestor-page metadata is
    // unusually large. The frontend can still select the stable anchor and
    // request its hierarchy page on demand.
    if let Some(mut result) = matches.into_iter().next() {
        result.reveal_pages.clear();
        let fallback = ServerEvent::Viewport(ViewportEvent::SearchResults {
            query,
            offset,
            total,
            matches: vec![result],
            has_more,
        });
        if queue_bounded_event(state, request_id.as_deref(), fallback) {
            return;
        }
    }

    warn!(
        "[viewport-data-channel] dropping search result page because one result exceeds the application message limit"
    );
}

fn queue_scene_children_page(
    state: &mut ApplicationSessionState,
    request_id: Option<String>,
    page: viewport_protocol::SceneChildrenPage,
) {
    let event = ServerEvent::Viewport(ViewportEvent::SceneChildren { page: page.clone() });
    if queue_bounded_event(state, request_id.as_deref(), event) {
        return;
    }

    // Keep the logical parent/page identity unchanged while splitting only
    // the transport payload. The frontend merges repeated responses for the
    // same page, so callers do not need a new protocol field or a second
    // request. This also keeps ordered DataChannel sequence numbers intact.
    if page.nodes.len() > 1 {
        let split = page.nodes.len() / 2;
        let viewport_protocol::SceneChildrenPage {
            parent,
            page,
            page_size,
            total,
            nodes,
        } = page;
        let mut tail = nodes;
        let head = tail.split_off(split);
        let first = viewport_protocol::SceneChildrenPage {
            parent: parent.clone(),
            page,
            page_size,
            total,
            nodes: tail,
        };
        let second = viewport_protocol::SceneChildrenPage {
            parent,
            page,
            page_size,
            total,
            nodes: head,
        };
        queue_scene_children_page(state, request_id.clone(), first);
        queue_scene_children_page(state, request_id, second);
        return;
    }

    warn!(
        "[viewport-data-channel] dropping scene child node because it exceeds the application message limit"
    );
}

fn queue_bounded_event(
    state: &mut ApplicationSessionState,
    request_id: Option<&str>,
    event: ServerEvent,
) -> bool {
    let envelope = next_server_envelope(state, request_id, event);
    if encoded_size(&envelope).is_some_and(|size| size <= MAX_APPLICATION_MESSAGE_BYTES) {
        state.pending_server_events.push_back(envelope);
        true
    } else {
        // The envelope was only provisional; do not create a sequence gap.
        state.server_sequence = state.server_sequence.saturating_sub(1);
        false
    }
}

fn queue_snapshot(
    state: &mut ApplicationSessionState,
    request_id: Option<String>,
    snapshot: ViewportReadModel,
    session_snapshot: bool,
) {
    let event = if session_snapshot {
        ServerEvent::Session(SessionEvent::Snapshot {
            state: snapshot.clone(),
        })
    } else {
        ServerEvent::Viewport(ViewportEvent::Snapshot {
            state: snapshot.clone(),
        })
    };
    let envelope = next_server_envelope(state, request_id.as_deref(), event);
    if encoded_size(&envelope).is_some_and(|size| size <= MAX_APPLICATION_MESSAGE_BYTES) {
        state.pending_server_events.push_back(envelope);
        return;
    }

    // The provisional envelope consumed a sequence number. Reuse it as the
    // base for the chunked form so the ordered stream has no sequence gap.
    state.server_sequence = state.server_sequence.saturating_sub(1);
    let snapshot_id = request_id
        .clone()
        .unwrap_or_else(|| format!("snapshot-{}", state.server_sequence));
    let mut chunk_size = INITIAL_SNAPSHOT_CHUNK_PRIMS.max(1);

    loop {
        let chunks: Vec<ServerEventEnvelope> = snapshot
            .scene
            .prims
            .chunks(chunk_size)
            .enumerate()
            .map(|(chunk_index, prims)| {
                let mut chunk_state = snapshot.clone();
                chunk_state.scene.prims = prims.to_vec();
                let chunk_count = snapshot.scene.prims.len().div_ceil(chunk_size);
                next_server_envelope(
                    state,
                    request_id.as_deref(),
                    ServerEvent::Session(SessionEvent::SnapshotChunk {
                        snapshot_id: snapshot_id.clone(),
                        chunk_index: chunk_index as u32,
                        chunk_count: chunk_count as u32,
                        state: chunk_state,
                    }),
                )
            })
            .collect();

        if chunks.iter().all(|envelope| {
            encoded_size(envelope).is_some_and(|size| size <= MAX_APPLICATION_MESSAGE_BYTES)
        }) {
            info!(
                "[viewport-data-channel] queued snapshot {} in {} chunks ({} prims)",
                snapshot_id,
                chunks.len(),
                snapshot.scene.prims.len()
            );
            state.pending_server_events.extend(chunks);
            return;
        }

        // Roll back the provisional chunk sequence range before retrying with
        // smaller chunks. One prim per message is the final practical bound.
        state.server_sequence = state.server_sequence.saturating_sub(chunks.len() as u64);
        if chunk_size == 1 {
            error!("[viewport-data-channel] one prim still exceeds the application message limit");
            let envelope = next_server_envelope(
                state,
                request_id.as_deref(),
                ServerEvent::Session(SessionEvent::Snapshot { state: snapshot }),
            );
            state.pending_server_events.push_back(envelope);
            return;
        }
        chunk_size = (chunk_size / 2).max(1);
    }
}

fn next_server_envelope(
    state: &mut ApplicationSessionState,
    request_id: Option<&str>,
    event: ServerEvent,
) -> ServerEventEnvelope {
    let sequence = state.server_sequence;
    state.server_sequence = state.server_sequence.saturating_add(1);
    match request_id {
        Some(request_id) => {
            let mut envelope = ServerEventEnvelope::for_request(
                state.session_id.clone(),
                sequence,
                request_id,
                event,
            );
            envelope.causation_id = Some(request_id.to_owned());
            envelope
        }
        None => ServerEventEnvelope::new(state.session_id.clone(), sequence, event),
    }
}

fn encoded_size(envelope: &ServerEventEnvelope) -> Option<usize> {
    encode_server_json_line(envelope)
        .ok()
        .map(|json| json.len())
}

fn flush_pending_server_events(channel: &WebRTCDataChannel, state: &mut ApplicationSessionState) {
    while let Some(envelope) = state.pending_server_events.front().cloned() {
        let result = encode_server_json_line(&envelope)
            .map_err(|error| error.to_string())
            .and_then(|json| {
                channel
                    .send_string_full(Some(&json))
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(()) => {
                state.pending_server_events.pop_front();
            }
            Err(error) => {
                warn!("[viewport-data-channel] application event send failed: {error}");
                if error.to_ascii_lowercase().contains("too large") {
                    // Browser/GStreamer limits can be stricter than the local
                    // JSON size guard. Replace the rejected head with a
                    // same-sequence compact rejection so ordered consumers do
                    // not observe a sequence gap and block forever.
                    if let Some(oversized) = state.pending_server_events.pop_front() {
                        let rejection_id = oversized
                            .request_id
                            .clone()
                            .unwrap_or_else(|| format!("server-sequence-{}", oversized.sequence));
                        let mut replacement = oversized;
                        replacement.event = ServerEvent::Viewport(ViewportEvent::CommandRejected {
                            request_id: rejection_id,
                            reason: "application event exceeded the DataChannel message limit"
                                .to_owned(),
                        });
                        state.pending_server_events.push_front(replacement);
                    }
                    continue;
                }
                break;
            }
        }
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
    fn recent_request_ids_are_deduplicated() {
        let session = ApplicationSession::new(
            SessionId::new("session-1"),
            ViewportReadModel::unloaded("stage.usda"),
            RenderServerInterface::default(),
        );
        let mut state = session.state.lock().unwrap();

        assert!(remember_request_id(&mut state, "request-1".to_owned()));
        assert!(!remember_request_id(&mut state, "request-1".to_owned()));
    }

    #[test]
    fn large_snapshots_are_chunked_into_bounded_ordered_events() {
        let session = ApplicationSession::new(
            SessionId::new("session-1"),
            ViewportReadModel::unloaded("stage.usda"),
            RenderServerInterface::default(),
        );
        let mut state = session.state.lock().unwrap();
        let mut snapshot = ViewportReadModel::unloaded("stage.usda");
        snapshot.stage.loaded = true;
        snapshot.scene.prims = (0..2_745)
            .map(|index| viewport_protocol::PrimNodeReadModel {
                anchor: viewport_protocol::SceneAnchor::active_session(format!(
                    "/World/Prim{index}"
                )),
                parent: None,
                label: format!("Prim {index}"),
                visible: true,
                has_children: false,
            })
            .collect();

        queue_server_event_for_request(
            &mut state,
            Some("snapshot-request".to_owned()),
            ServerEvent::Session(SessionEvent::Snapshot { state: snapshot }),
        );

        assert!(state.pending_server_events.len() > 1);
        let mut expected_sequence = 1;
        let mut expected_index = 0;
        let mut expected_count = None;
        for envelope in &state.pending_server_events {
            assert_eq!(envelope.sequence, expected_sequence);
            expected_sequence += 1;
            let ServerEvent::Session(SessionEvent::SnapshotChunk {
                snapshot_id,
                chunk_index,
                chunk_count,
                ..
            }) = &envelope.event
            else {
                panic!("large snapshots must be sent as snapshot chunks");
            };
            assert_eq!(snapshot_id, "snapshot-request");
            assert_eq!(*chunk_index, expected_index);
            expected_index += 1;
            expected_count.get_or_insert(*chunk_count);
            assert_eq!(expected_count, Some(*chunk_count));
            assert!(encoded_size(envelope).unwrap() <= MAX_APPLICATION_MESSAGE_BYTES);
        }
        assert_eq!(expected_count, Some(expected_index));
    }

    #[test]
    fn oversized_scene_child_pages_are_split_without_changing_page_identity() {
        let session = ApplicationSession::new(
            SessionId::new("session-1"),
            ViewportReadModel::unloaded("stage.usda"),
            RenderServerInterface::default(),
        );
        let mut state = session.state.lock().unwrap();
        let parent = viewport_protocol::SceneAnchor::active_session(
            "/World/Very/Deep/Animated/Geometry/geom",
        );
        let nodes = (0..128)
            .map(|index| viewport_protocol::PrimNodeReadModel {
                anchor: viewport_protocol::SceneAnchor::active_session(format!(
                    "/World/Very/Deep/Animated/Geometry/geom/child-{index}-with-a-long-name"
                )),
                parent: Some(parent.clone()),
                label: format!("child-{index}"),
                visible: true,
                has_children: false,
            })
            .collect();

        queue_server_event_for_request(
            &mut state,
            Some("children-request".to_owned()),
            ServerEvent::Viewport(ViewportEvent::SceneChildren {
                page: viewport_protocol::SceneChildrenPage {
                    parent: Some(parent.clone()),
                    page: 0,
                    page_size: viewport_protocol::DEFAULT_SCENE_PAGE_SIZE,
                    total: 128,
                    nodes,
                },
            }),
        );

        assert!(state.pending_server_events.len() > 1);
        let mut expected_sequence = 1;
        let mut received_nodes = 0;
        for envelope in &state.pending_server_events {
            assert_eq!(envelope.sequence, expected_sequence);
            expected_sequence += 1;
            assert!(encoded_size(envelope).unwrap() <= MAX_APPLICATION_MESSAGE_BYTES);
            let ServerEvent::Viewport(ViewportEvent::SceneChildren { page }) = &envelope.event
            else {
                panic!("oversized child pages must remain child-page events");
            };
            assert_eq!(page.parent.as_ref(), Some(&parent));
            assert_eq!(page.page, 0);
            assert_eq!(page.page_size, viewport_protocol::DEFAULT_SCENE_PAGE_SIZE);
            assert_eq!(page.total, 128);
            received_nodes += page.nodes.len();
        }
        assert_eq!(received_nodes, 128);
    }
}
