use std::collections::{HashMap, HashSet, VecDeque};
use viewport_protocol::{
    AuthorizationPolicy, ClientCommandEnvelope, InputCommand, PointerMotion, RuntimeManifest,
    SemanticSyncStatus, SessionId, ViewportCommandEnvelope, ViewportEventEnvelope, ViewportMetrics,
    ViewportReadModel,
};

use super::types::SemanticSyncRequest;

#[derive(Debug, Default)]
pub(super) struct PendingMessages {
    pub(super) commands: VecDeque<ClientCommandEnvelope>,
    pub(super) viewport_commands: VecDeque<ViewportCommandEnvelope>,
    pub(super) input_commands: VecDeque<InputCommand>,
    pub(super) latest_pointer_motion: Option<PointerMotion>,
    pub(super) last_pointer_sequence: u64,
    pub(super) input_reset: bool,
    pub(super) viewport_events: VecDeque<ViewportEventEnvelope>,
    pub(super) semantic_sync_requests: VecDeque<SemanticSyncRequest>,
    pub(super) semantic_sync_control_requests: HashMap<SessionId, SemanticSyncRequest>,
    pub(super) closed_sessions: HashSet<SessionId>,
    pub(super) latest_snapshot: Option<ViewportReadModel>,
    pub(super) authorization: AuthorizationPolicy,
    pub(super) semantic_sync_statuses: HashMap<SessionId, SemanticSyncStatus>,
    pub(super) runtime_manifest: Option<RuntimeManifest>,
    pub(super) runtime_blobs: HashMap<String, Vec<u8>>,
    pub(super) pending_stream_configuration: Option<ViewportMetrics>,
    // A stream configuration can originate from a newly connected WebView,
    // whose local generation counter starts over at one. Keep this sequence
    // with the long-lived server interface rather than in a WebRTC session so
    // a reconnect cannot make the Bevy target and frame router disagree.
    pub(super) latest_stream_generation: u64,
}
