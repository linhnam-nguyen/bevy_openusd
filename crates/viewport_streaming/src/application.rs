//! Shared application bus between the Bevy viewport and WebRTC sessions.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};

use viewport_protocol::{
    AuthorizationPolicy, ClientCommandEnvelope, InputCommand, PointerMotion, RuntimeManifest,
    SemanticSyncOperation, SemanticSyncStatus, SessionId, ViewportCommandEnvelope, ViewportEvent,
    ViewportEventEnvelope, ViewportMetrics, ViewportReadModel, validate_runtime_blob_id,
};

const MAX_PENDING_MESSAGES: usize = 256;

#[derive(Debug, Default)]
struct PendingMessages {
    commands: VecDeque<ClientCommandEnvelope>,
    viewport_commands: VecDeque<ViewportCommandEnvelope>,
    input_commands: VecDeque<InputCommand>,
    latest_pointer_motion: Option<PointerMotion>,
    last_pointer_sequence: u64,
    input_reset: bool,
    viewport_events: VecDeque<ViewportEventEnvelope>,
    semantic_sync_requests: VecDeque<SemanticSyncRequest>,
    latest_snapshot: Option<ViewportReadModel>,
    authorization: AuthorizationPolicy,
    semantic_sync_statuses: HashMap<SessionId, SemanticSyncStatus>,
    runtime_manifest: Option<RuntimeManifest>,
    runtime_blobs: HashMap<String, Vec<u8>>,
    pending_stream_configuration: Option<ViewportMetrics>,
    // A stream configuration can originate from a newly connected WebView,
    // whose local generation counter starts over at one. Keep this sequence
    // with the long-lived server interface rather than in a WebRTC session so
    // a reconnect cannot make the Bevy target and frame router disagree.
    latest_stream_generation: u64,
}

/// Transport-neutral application boundary shared across the ECS and WebRTC
/// threads. It contains no Bevy, GStreamer, Tokio, or DOM objects.
#[derive(Debug, Clone, Default)]
pub struct RenderServerInterface {
    pending: Arc<Mutex<PendingMessages>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderServerPortError {
    QueueClosed,
    QueueFull,
    InvalidPayload,
}

/// Server-enriched semantic-sync request queued between the transport and the
/// application runtime. The authorization policy comes from the established
/// server session, never from the client command payload.
#[derive(Debug, Clone, PartialEq)]
pub enum SemanticSyncRequestKind {
    Client(SemanticSyncOperation),
    AuthorizationChanged,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticSyncRequest {
    pub request_id: String,
    pub session_id: SessionId,
    pub client_name: String,
    pub authorization: AuthorizationPolicy,
    pub kind: SemanticSyncRequestKind,
}

impl RenderServerInterface {
    pub fn pending_command_count(&self) -> usize {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .commands
            .len()
    }

    pub fn pending_event_count(&self) -> usize {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .viewport_events
            .len()
    }

    pub fn pending_input_count(&self) -> usize {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .input_commands
            .len()
    }

    pub fn submit(&self, command: ClientCommandEnvelope) -> Result<(), RenderServerPortError> {
        command
            .validate()
            .map_err(|_| RenderServerPortError::InvalidPayload)?;

        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        if pending.commands.len() >= MAX_PENDING_MESSAGES {
            return Err(RenderServerPortError::QueueFull);
        }
        pending.commands.push_back(command);
        Ok(())
    }

    pub fn pop_command(&self) -> Option<ClientCommandEnvelope> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .commands
            .pop_front()
    }

    pub fn submit_semantic_sync_request(
        &self,
        request: SemanticSyncRequest,
    ) -> Result<(), RenderServerPortError> {
        if request.request_id.trim().is_empty()
            || request.client_name.trim().is_empty()
            || request.session_id.validate().is_err()
            || request.authorization.validate().is_err()
        {
            return Err(RenderServerPortError::InvalidPayload);
        }
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        if pending.semantic_sync_requests.len() >= MAX_PENDING_MESSAGES {
            return Err(RenderServerPortError::QueueFull);
        }
        pending.semantic_sync_requests.push_back(request);
        Ok(())
    }

    pub fn pop_semantic_sync_request(&self) -> Option<SemanticSyncRequest> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .semantic_sync_requests
            .pop_front()
    }

    pub fn submit_viewport_command(
        &self,
        command: ViewportCommandEnvelope,
    ) -> Result<(), RenderServerPortError> {
        command
            .validate()
            .map_err(|_| RenderServerPortError::InvalidPayload)?;
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        if pending.viewport_commands.len() >= MAX_PENDING_MESSAGES {
            return Err(RenderServerPortError::QueueFull);
        }
        pending.viewport_commands.push_back(command);
        Ok(())
    }

    pub fn pop_viewport_command(&self) -> Option<ViewportCommandEnvelope> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .viewport_commands
            .pop_front()
    }

    /// Queues reliable input state and keeps only the newest motion packet.
    /// The latter is deliberately replaceable so pointer traffic cannot
    /// block semantic commands or make the camera process stale deltas.
    pub fn submit_input(&self, command: InputCommand) -> Result<(), RenderServerPortError> {
        command
            .validate()
            .map_err(|_| RenderServerPortError::InvalidPayload)?;

        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        if let InputCommand::PointerMotion(motion) = command {
            if motion.sequence <= pending.last_pointer_sequence {
                return Ok(());
            }
            pending.last_pointer_sequence = motion.sequence;
            pending.latest_pointer_motion = Some(motion);
            return Ok(());
        }
        if pending.input_commands.len() >= MAX_PENDING_MESSAGES {
            return Err(RenderServerPortError::QueueFull);
        }
        pending.input_commands.push_back(command);
        Ok(())
    }

    pub fn pop_input(&self) -> Option<InputCommand> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .input_commands
            .pop_front()
    }

    /// Assigns a server-monotonic generation and queues the newest validated
    /// viewport request for the Bevy main thread. The transport callback never
    /// mutates `Assets<Image>` directly.
    ///
    /// Browser/WebView sessions are allowed to restart their local counter.
    /// The returned metrics must therefore be echoed to both the encoder frame
    /// router and the client, while Bevy receives the identical request here.
    pub fn submit_stream_configuration(
        &self,
        mut metrics: ViewportMetrics,
    ) -> Result<ViewportMetrics, RenderServerPortError> {
        metrics
            .validate()
            .map_err(|_| RenderServerPortError::InvalidPayload)?;
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        let next_generation = pending.latest_stream_generation.saturating_add(1);
        metrics.generation = metrics.generation.max(next_generation);
        pending.latest_stream_generation = metrics.generation;
        pending.pending_stream_configuration = Some(metrics.clone());
        Ok(metrics)
    }

    pub fn take_stream_configuration(&self) -> Option<ViewportMetrics> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .pending_stream_configuration
            .take()
    }

    pub fn take_latest_pointer_motion(&self) -> Option<PointerMotion> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .latest_pointer_motion
            .take()
    }

    /// Clears all remote input when a peer/channel disappears. The Bevy
    /// adapter observes the reset marker on its next update tick.
    pub fn clear_remote_input(&self) {
        let mut pending = self
            .pending
            .lock()
            .expect("render-server interface queue is not poisoned");
        pending.input_commands.clear();
        pending.latest_pointer_motion = None;
        pending.last_pointer_sequence = 0;
        pending.input_reset = true;
    }

    pub fn take_input_reset(&self) -> bool {
        let mut pending = self
            .pending
            .lock()
            .expect("render-server interface queue is not poisoned");
        let reset = pending.input_reset;
        pending.input_reset = false;
        reset
    }

    pub fn publish_viewport_event(
        &self,
        mut event: ViewportEventEnvelope,
    ) -> Result<(), RenderServerPortError> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        if pending.viewport_events.len() >= MAX_PENDING_MESSAGES {
            return Err(RenderServerPortError::QueueFull);
        }

        if let ViewportEvent::Snapshot { state } = &mut event.event {
            state.stage.display_name = safe_display_name(&state.stage.display_name);
            pending.latest_snapshot = Some(state.clone());
        }
        pending.viewport_events.push_back(event);
        Ok(())
    }

    pub fn pop_viewport_event(&self) -> Option<ViewportEventEnvelope> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .viewport_events
            .pop_front()
    }

    pub fn requeue_viewport_event_front(
        &self,
        event: ViewportEventEnvelope,
    ) -> Result<(), RenderServerPortError> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        if pending.viewport_events.len() >= MAX_PENDING_MESSAGES {
            return Err(RenderServerPortError::QueueFull);
        }
        pending.viewport_events.push_front(event);
        Ok(())
    }

    /// Returns the newest state and discards historical events. A newly
    /// connected session receives the current snapshot first; replaying
    /// pre-handshake lifecycle events could regress it to an older state.
    pub fn take_latest_snapshot(&self, fallback: ViewportReadModel) -> ViewportReadModel {
        let mut pending = self
            .pending
            .lock()
            .expect("render-server interface queue is not poisoned");
        pending.viewport_events.clear();
        pending.latest_snapshot.clone().unwrap_or(fallback)
    }

    /// Publishes the current server-owned runtime inventory. The manifest is
    /// kept behind this application boundary and is filtered by each session
    /// authorization policy before it is sent to a client.
    pub fn publish_runtime_manifest(
        &self,
        manifest: RuntimeManifest,
    ) -> Result<(), RenderServerPortError> {
        manifest
            .validate()
            .map_err(|_| RenderServerPortError::InvalidPayload)?;
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        pending.runtime_manifest = Some(manifest);
        Ok(())
    }

    /// Replaces the server-owned authorization policy. Existing sessions
    /// observe the change at their next reliable event flush and receive an
    /// explicit policy event before requesting another runtime revision.
    pub fn publish_authorization_policy(
        &self,
        authorization: AuthorizationPolicy,
    ) -> Result<(), RenderServerPortError> {
        authorization
            .validate()
            .map_err(|_| RenderServerPortError::InvalidPayload)?;
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        pending.authorization = authorization;
        Ok(())
    }

    pub fn authorization_policy(&self) -> AuthorizationPolicy {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .authorization
            .clone()
    }

    /// Publishes the newest semantic-sync lifecycle state for one session.
    /// Credentials and remote transport details remain outside this bus.
    pub fn publish_semantic_sync_status(
        &self,
        session_id: SessionId,
        status: SemanticSyncStatus,
    ) -> Result<(), RenderServerPortError> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        pending.semantic_sync_statuses.insert(session_id, status);
        Ok(())
    }

    pub fn semantic_sync_status(&self, session_id: &SessionId) -> Option<SemanticSyncStatus> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .semantic_sync_statuses
            .get(session_id)
            .cloned()
    }

    pub fn clear_semantic_sync_status(&self, session_id: &SessionId) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.semantic_sync_statuses.remove(session_id);
        }
    }

    /// Atomically replaces the runtime manifest and exactly the verified blob
    /// bytes referenced by it. A failed publication leaves the previous bundle
    /// untouched, so a client cannot observe a partial revision.
    pub fn publish_runtime_delivery(
        &self,
        manifest: RuntimeManifest,
        blobs: Vec<(String, Vec<u8>)>,
    ) -> Result<(), RenderServerPortError> {
        manifest
            .validate()
            .map_err(|_| RenderServerPortError::InvalidPayload)?;
        let mut blob_map = HashMap::with_capacity(blobs.len());
        for (blob_id, bytes) in blobs {
            if validate_runtime_blob_id(&blob_id).is_err()
                || blob_map.insert(blob_id, bytes).is_some()
            {
                return Err(RenderServerPortError::InvalidPayload);
            }
        }
        let references = manifest.references();
        if references.len() != blob_map.len()
            || references.iter().any(|reference| {
                blob_map
                    .get(&reference.blob_id)
                    .is_none_or(|bytes| bytes.len() as u64 != reference.byte_size)
            })
        {
            return Err(RenderServerPortError::InvalidPayload);
        }

        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        pending.runtime_manifest = Some(manifest);
        pending.runtime_blobs = blob_map;
        Ok(())
    }

    pub fn clear_runtime_delivery(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.runtime_manifest = None;
            pending.runtime_blobs.clear();
        }
    }

    /// Publishes verified bytes for one content-addressed runtime object.
    /// Filesystem/object-store adapters remain outside this transport crate;
    /// they provide the bytes only after validating the content address.
    pub fn publish_runtime_blob(
        &self,
        blob_id: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Result<(), RenderServerPortError> {
        let blob_id = blob_id.into();
        if validate_runtime_blob_id(&blob_id).is_err() {
            return Err(RenderServerPortError::InvalidPayload);
        }
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        pending.runtime_blobs.insert(blob_id, bytes);
        Ok(())
    }

    pub fn runtime_manifest(&self) -> Option<RuntimeManifest> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .runtime_manifest
            .clone()
    }

    pub fn runtime_blob(&self, blob_id: &str) -> Option<Vec<u8>> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .runtime_blobs
            .get(blob_id)
            .cloned()
    }
}

fn safe_display_name(value: &str) -> String {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("remote-stage")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewport_protocol::{
        InputCommand, PointerMotion, SemanticSyncOperation, SemanticSyncPhase, SemanticSyncStatus,
        SessionId, ViewportCommand,
    };

    #[test]
    fn stream_generation_remains_monotonic_across_webview_reconnects() {
        let interface = RenderServerInterface::default();

        let first = interface
            .submit_stream_configuration(ViewportMetrics::default())
            .expect("initial configuration should be valid");
        assert_eq!(first.generation, 1);
        assert_eq!(interface.take_stream_configuration(), Some(first));

        let resized = interface
            .submit_stream_configuration(ViewportMetrics {
                generation: 2,
                ..ViewportMetrics::default()
            })
            .expect("resized configuration should be valid");
        assert_eq!(resized.generation, 2);
        assert_eq!(interface.take_stream_configuration(), Some(resized));

        // A Tauri WebView refresh starts its local counter at one again. The
        // server must advance from the prior resize instead of reusing one.
        let refreshed = interface
            .submit_stream_configuration(ViewportMetrics::default())
            .expect("reconnected configuration should be valid");
        assert_eq!(refreshed.generation, 3);
        assert_eq!(interface.take_stream_configuration(), Some(refreshed));
    }

    #[test]
    fn snapshot_display_name_is_reduced_to_a_basename() {
        let interface = RenderServerInterface::default();
        interface
            .publish_viewport_event(ViewportEventEnvelope::new(
                None,
                ViewportEvent::Snapshot {
                    state: ViewportReadModel::unloaded("/private/stages/Kitchen_set.usdz"),
                },
            ))
            .unwrap();

        assert_eq!(
            interface
                .take_latest_snapshot(ViewportReadModel::unloaded("fallback.usda"))
                .stage
                .display_name,
            "Kitchen_set.usdz"
        );
    }

    #[test]
    fn event_history_is_cleared_when_a_session_takes_a_snapshot() {
        let interface = RenderServerInterface::default();
        let snapshot = ViewportReadModel::unloaded("stage.usda");
        interface
            .publish_viewport_event(ViewportEventEnvelope::new(
                None,
                ViewportEvent::Snapshot {
                    state: snapshot.clone(),
                },
            ))
            .unwrap();

        assert_eq!(interface.pending_event_count(), 1);
        assert_eq!(interface.take_latest_snapshot(snapshot.clone()), snapshot);
        assert_eq!(interface.pending_event_count(), 0);
    }

    #[test]
    fn viewport_command_queue_rejects_invalid_correlation_metadata() {
        let interface = RenderServerInterface::default();
        let command = ViewportCommandEnvelope::new("", ViewportCommand::RequestSnapshot);

        assert_eq!(
            interface.submit_viewport_command(command),
            Err(RenderServerPortError::InvalidPayload)
        );
        assert_eq!(interface.pending_command_count(), 0);
    }

    #[test]
    fn an_event_can_be_put_back_after_a_failed_transport_send() {
        let interface = RenderServerInterface::default();
        let event = ViewportEventEnvelope::new(
            Some("request-1".to_owned()),
            ViewportEvent::PhysicsChanged { running: true },
        );
        interface
            .publish_viewport_event(event.clone())
            .expect("event should enter the bounded queue");
        let removed = interface
            .pop_viewport_event()
            .expect("event should be available for transport");
        interface
            .requeue_viewport_event_front(removed)
            .expect("failed sends must be recoverable");

        assert_eq!(interface.pop_viewport_event(), Some(event));
    }

    #[test]
    fn pointer_motion_keeps_only_the_newest_valid_packet() {
        let interface = RenderServerInterface::default();
        let motion = |sequence| PointerMotion {
            sequence,
            dx_css_pixels: sequence as f32,
            dy_css_pixels: 0.0,
            wheel_x: 0.0,
            wheel_y: 0.0,
            viewport_css_width: 800.0,
            viewport_css_height: 600.0,
            stream_generation: 0,
        };

        interface
            .submit_input(InputCommand::PointerMotion(motion(1)))
            .unwrap();
        interface
            .submit_input(InputCommand::PointerMotion(motion(2)))
            .unwrap();
        interface
            .submit_input(InputCommand::PointerMotion(motion(1)))
            .unwrap();

        assert_eq!(interface.take_latest_pointer_motion().unwrap().sequence, 2);
        assert!(interface.take_latest_pointer_motion().is_none());
    }

    #[test]
    fn reliable_input_state_is_preserved_in_order() {
        let interface = RenderServerInterface::default();
        interface
            .submit_input(InputCommand::ReleaseAll(
                viewport_protocol::ReleaseAllInput { sequence: 1 },
            ))
            .unwrap();
        assert!(matches!(
            interface.pop_input(),
            Some(InputCommand::ReleaseAll(_))
        ));
    }

    #[test]
    fn runtime_delivery_registry_rejects_invalid_ids_and_keeps_valid_payloads() {
        let interface = RenderServerInterface::default();
        assert_eq!(
            interface.publish_runtime_blob("../outside", vec![1, 2, 3]),
            Err(RenderServerPortError::InvalidPayload)
        );

        let blob_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        interface
            .publish_runtime_blob(blob_id, vec![1, 2, 3])
            .unwrap();
        assert_eq!(interface.runtime_blob(blob_id), Some(vec![1, 2, 3]));
    }

    #[test]
    fn authorization_policy_is_validated_and_replaceable() {
        let interface = RenderServerInterface::default();
        assert_eq!(
            interface.authorization_policy(),
            AuthorizationPolicy::default()
        );

        let mut policy = AuthorizationPolicy::default();
        policy.semantic_property_scope = viewport_protocol::SemanticPropertyScope::All;
        interface
            .publish_authorization_policy(policy.clone())
            .unwrap();

        assert_eq!(interface.authorization_policy(), policy);
        assert_eq!(
            interface.publish_authorization_policy(AuthorizationPolicy {
                allowed_delivery_modes: Vec::new(),
                ..AuthorizationPolicy::default()
            }),
            Err(RenderServerPortError::InvalidPayload)
        );
        assert_eq!(interface.authorization_policy(), policy);
    }

    #[test]
    fn semantic_sync_status_is_replaced_per_session_and_can_be_cleared() {
        let interface = RenderServerInterface::default();
        let session_id = SessionId::new("session-sync");
        let ready = SemanticSyncStatus::ready("working-7".to_owned(), "hash-1".to_owned());
        interface
            .publish_semantic_sync_status(session_id.clone(), ready.clone())
            .unwrap();
        assert_eq!(interface.semantic_sync_status(&session_id), Some(ready));

        let stale = SemanticSyncStatus::phase(
            SemanticSyncPhase::Stale,
            Some("authorization_changed".to_owned()),
        );
        interface
            .publish_semantic_sync_status(session_id.clone(), stale.clone())
            .unwrap();
        assert_eq!(interface.semantic_sync_status(&session_id), Some(stale));

        interface.clear_semantic_sync_status(&session_id);
        assert!(interface.semantic_sync_status(&session_id).is_none());
    }

    #[test]
    fn semantic_sync_requests_preserve_server_context_and_reject_invalid_context() {
        let interface = RenderServerInterface::default();
        let request = SemanticSyncRequest {
            request_id: "sync-1".to_owned(),
            session_id: SessionId::new("session-sync"),
            client_name: "native-client".to_owned(),
            authorization: AuthorizationPolicy::default(),
            operation: SemanticSyncOperation::Provision,
        };
        interface
            .submit_semantic_sync_request(request.clone())
            .expect("valid semantic-sync request should queue");
        assert_eq!(interface.pop_semantic_sync_request(), Some(request.clone()));

        let invalid = SemanticSyncRequest {
            client_name: String::new(),
            ..request
        };
        assert_eq!(
            interface.submit_semantic_sync_request(invalid),
            Err(RenderServerPortError::InvalidPayload)
        );
    }
}
