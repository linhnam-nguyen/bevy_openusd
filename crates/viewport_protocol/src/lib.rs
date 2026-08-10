//! UI-neutral contract for controlling and observing a USDHub viewport.
//!
//! This crate deliberately has no dependency on Bevy, egui, Frost, Tauri, or
//! OpenUSD. The native viewport and every UI client communicate in terms of
//! stable scene anchors, commands, events, and read-model snapshots instead
//! of engine-local entities or asset handles.

use serde::{Deserialize, Serialize};

/// Current wire/API version supported by this contract.
pub const PROTOCOL_VERSION: u16 = 1;

/// A client-generated ID used to correlate a command with a rejection or a
/// later acknowledgement event. A string keeps the type transport-neutral.
pub type RequestId = String;

/// A viewport session known to a host product.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

/// Stable, renderer-neutral identity for a logical USD target.
///
/// A prim path can correspond to more than one runtime entity. The viewport
/// resolves that internally; `instance_context` is reserved for the future
/// case where a UI needs to select an individual instancer occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SceneAnchor {
    pub session_id: Option<SessionId>,
    pub prim_path: String,
    pub instance_context: Option<String>,
}

impl SceneAnchor {
    /// Creates an anchor for a USD prim in the active viewport session.
    pub fn active_session(prim_path: impl Into<String>) -> Self {
        Self {
            session_id: None,
            prim_path: prim_path.into(),
            instance_context: None,
        }
    }
}

/// A camera source selected by a product UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraSource {
    Arcball,
    Authored { prim_path: String },
}

/// A Bevy-rendered overlay controlled by a UI client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayKind {
    GroundGrid,
    WorldAxes,
    PrimMarkers,
    Skeleton,
    Physics,
    Colliders,
    Wireframe,
}

/// A focused camera transition requested by a UI client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusMode {
    FrameTarget,
    FlyToTarget,
}

/// A logical USD prim made available to a product UI tree.
///
/// `anchor` is opaque to the UI: it lets the viewport preserve exact tree-row
/// behavior even when one USD path is expanded into more than one runtime
/// entity. `parent` is absent only for scene roots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrimNodeReadModel {
    pub anchor: SceneAnchor,
    pub parent: Option<SceneAnchor>,
    pub label: String,
    pub visible: bool,
    pub has_children: bool,
}

/// Hierarchy data used by the product prim tree.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneReadModel {
    pub prims: Vec<PrimNodeReadModel>,
}

/// Rendering settings applied while projecting USD curves and points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurveTuning {
    pub default_radius: f32,
    pub ring_segments: u32,
    pub point_scale: f32,
}

/// The UI-neutral commands accepted by the viewport runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ViewportCommand {
    RequestSnapshot,
    ReloadSession,
    SelectTarget {
        target: Option<SceneAnchor>,
    },
    FocusTarget {
        target: SceneAnchor,
        mode: FocusMode,
    },
    SetSubtreeVisibility {
        target: SceneAnchor,
        visible: bool,
    },
    SetVariantSelection {
        prim_path: String,
        set_name: String,
        option: String,
    },
    ResetVariantSelection {
        prim_path: String,
        set_name: String,
    },
    SetCameraSource {
        source: CameraSource,
    },
    SetPlayback {
        playing: bool,
    },
    Seek {
        seconds: f64,
    },
    SetOverlay {
        overlay: OverlayKind,
        enabled: bool,
    },
    SetPrimMarkerBias {
        bias: f32,
    },
    SetLightIntensity {
        scale: f32,
    },
    SetCurveTuning {
        tuning: CurveTuning,
    },
    SetPhysicsRunning {
        running: bool,
    },
}

/// A command plus the correlation information needed across process bounds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewportCommandEnvelope {
    pub protocol_version: u16,
    pub request_id: RequestId,
    pub command: ViewportCommand,
}

impl ViewportCommandEnvelope {
    /// Creates a versioned command envelope for the current protocol.
    pub fn new(request_id: impl Into<RequestId>, command: ViewportCommand) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            command,
        }
    }
}

/// Minimal stage state suitable for host UI status and reconnect recovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageReadModel {
    pub display_name: String,
    pub loaded: bool,
}

/// Coarse lifecycle state for a viewport stage load or reload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageLoadState {
    Idle,
    Loading,
    Ready,
    Failed { message: String },
}

/// Selection state reported by the viewport after it has been applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionReadModel {
    pub target: Option<SceneAnchor>,
}

/// Animation timeline state reported by the viewport.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineReadModel {
    pub seconds: f64,
    pub playing: bool,
    pub start_time_code: f64,
    pub end_time_code: f64,
    pub time_codes_per_second: f64,
}

/// Bevy presentation state visible to product UI controls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresentationReadModel {
    pub ground_grid: bool,
    pub world_axes: bool,
    pub prim_markers: bool,
    pub prim_marker_bias: f32,
    pub skeleton: bool,
    pub physics: bool,
    pub colliders: bool,
    pub wireframe: bool,
    pub light_intensity_scale: f32,
    pub curve_tuning: CurveTuning,
}

/// Recoverable viewport state sent at connection time and on resynchronization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewportReadModel {
    pub protocol_version: u16,
    pub stage: StageReadModel,
    pub scene: SceneReadModel,
    pub selection: SelectionReadModel,
    pub camera_source: CameraSource,
    pub timeline: TimelineReadModel,
    pub presentation: PresentationReadModel,
    pub physics_running: bool,
}

/// Changes emitted by the viewport after its authoritative state has changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ViewportEvent {
    Ready {
        protocol_version: u16,
    },
    Snapshot {
        state: ViewportReadModel,
    },
    StageLoadStateChanged {
        state: StageLoadState,
    },
    SelectionChanged {
        selection: SelectionReadModel,
    },
    CameraTransitionStarted {
        target: SceneAnchor,
        mode: FocusMode,
    },
    PrimVisibilityChanged {
        target: SceneAnchor,
        visible: bool,
    },
    CameraSourceChanged {
        source: CameraSource,
    },
    TimelineChanged {
        timeline: TimelineReadModel,
    },
    PresentationChanged {
        presentation: PresentationReadModel,
    },
    PhysicsChanged {
        running: bool,
    },
    CommandRejected {
        request_id: RequestId,
        reason: String,
    },
}

/// A viewport event with an optional origin command for correlation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewportEventEnvelope {
    pub protocol_version: u16,
    pub request_id: Option<RequestId>,
    pub event: ViewportEvent,
}

impl ViewportEventEnvelope {
    /// Creates an event envelope for the current protocol.
    pub fn new(request_id: Option<RequestId>, event: ViewportEvent) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            event,
        }
    }
}

/// A message exchanged over a transport boundary.
///
/// Native standard input/output uses this type as newline-delimited JSON:
/// every line contains exactly one serialized message. A client writes only
/// [`ViewportWireMessage::Command`] values to the viewport's standard input;
/// the viewport writes only [`ViewportWireMessage::Event`] values to standard
/// output. Keeping both directions explicit makes accidental log output or a
/// reversed connection immediately detectable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ViewportWireMessage {
    Command(ViewportCommandEnvelope),
    Event(ViewportEventEnvelope),
}

/// Serializes one transport message as a single JSON Lines record.
///
/// The returned string always has exactly one trailing line-feed. JSON escapes
/// embedded newlines in string values, so a reader can safely process this one
/// physical line at a time.
pub fn encode_json_line(message: &ViewportWireMessage) -> serde_json::Result<String> {
    let mut line = serde_json::to_string(message)?;
    line.push('\n');
    Ok(line)
}

/// Parses one JSON Lines record into a transport message.
///
/// Leading/trailing whitespace, including the record's line-feed, is accepted
/// so callers can pass a line directly from [`std::io::BufRead::read_line`].
pub fn decode_json_line(line: &str) -> serde_json::Result<ViewportWireMessage> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_envelopes_use_the_current_protocol_version() {
        let envelope = ViewportCommandEnvelope::new(
            "request-7",
            ViewportCommand::SelectTarget {
                target: Some(SceneAnchor::active_session("/World/Robot")),
            },
        );

        assert_eq!(envelope.protocol_version, PROTOCOL_VERSION);
        assert_eq!(envelope.request_id, "request-7");
    }

    #[test]
    fn scene_anchor_is_independent_of_runtime_entities() {
        let anchor = SceneAnchor::active_session("/World/Robot");

        assert_eq!(anchor.prim_path, "/World/Robot");
        assert_eq!(anchor.session_id, None);
        assert_eq!(anchor.instance_context, None);
    }

    #[test]
    fn json_lines_round_trip_a_versioned_command() {
        let message = ViewportWireMessage::Command(ViewportCommandEnvelope::new(
            "desktop-42",
            ViewportCommand::SetPlayback { playing: true },
        ));

        let line = encode_json_line(&message).expect("command should serialize");

        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        assert_eq!(
            decode_json_line(&line).expect("command should deserialize"),
            message
        );
    }

    #[test]
    fn json_lines_round_trip_an_event_with_an_embedded_newline() {
        let message = ViewportWireMessage::Event(ViewportEventEnvelope::new(
            Some("desktop-43".to_owned()),
            ViewportEvent::CommandRejected {
                request_id: "desktop-43".to_owned(),
                reason: "first line\nsecond line".to_owned(),
            },
        ));

        let line = encode_json_line(&message).expect("event should serialize");

        assert_eq!(line.matches('\n').count(), 1);
        assert!(line.contains("\\n"));
        assert_eq!(
            decode_json_line(line.trim_end()).expect("event should deserialize"),
            message
        );
    }
}
