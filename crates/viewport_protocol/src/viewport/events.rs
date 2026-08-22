use serde::{Deserialize, Serialize};

use crate::{PROTOCOL_VERSION, RequestId};

use super::commands::ViewportCommandEnvelope;
use super::editor::{EditorOperation, EditorPrimReadModel, EditorStateReadModel};
use super::read_models::{
    CameraSource, FocusMode, PresentationReadModel, SceneAnchor, SceneChildrenPage,
    SceneSearchMatch, SelectionReadModel, StageLoadState, TimelineReadModel,
    ViewerSettingsReadModel, ViewportReadModel,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
// Snapshot is intentionally an inline public protocol payload. Boxing it
// would change the Rust API and complicate every transport/reducer call site;
// the wire representation and ownership boundary are more important here.
#[allow(clippy::large_enum_variant)]
pub enum ViewportEvent {
    Ready {
        protocol_version: u16,
    },
    Snapshot {
        state: ViewportReadModel,
    },
    SceneChildren {
        page: SceneChildrenPage,
    },
    SearchResults {
        query: String,
        offset: u32,
        total: u32,
        matches: Vec<SceneSearchMatch>,
        has_more: bool,
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
    ViewerSettingsChanged {
        settings: ViewerSettingsReadModel,
    },
    PhysicsChanged {
        running: bool,
    },
    CommandRejected {
        request_id: RequestId,
        reason: String,
    },
    EditorCommandCompleted {
        operation: EditorOperation,
        changed_paths: Vec<String>,
        state: EditorStateReadModel,
    },
    RuntimeMutationBatchAccepted {
        source_id: String,
        sequence: u64,
        base_revision: u64,
        applied_operations: u32,
        changed_paths: Vec<String>,
        state: EditorStateReadModel,
    },
    EditorPrimState {
        prim: EditorPrimReadModel,
    },
    EditorStageExportChunk {
        export_id: String,
        chunk_index: u32,
        chunk_count: u32,
        content: String,
    },
}

/// Legacy event envelope retained byte/schema compatible with version 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewportEventEnvelope {
    pub protocol_version: u16,
    pub request_id: Option<RequestId>,
    pub event: ViewportEvent,
}

impl ViewportEventEnvelope {
    pub fn new(request_id: Option<RequestId>, event: ViewportEvent) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            event,
        }
    }
}

/// Legacy JSON Lines direction marker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ViewportWireMessage {
    Command(ViewportCommandEnvelope),
    Event(ViewportEventEnvelope),
}
