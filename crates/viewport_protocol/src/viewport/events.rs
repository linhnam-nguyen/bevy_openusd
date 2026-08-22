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
pub enum ViewportEvent {
    Ready {
        protocol_version: u16,
    },
    Snapshot {
        state: Box<ViewportReadModel>,
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

/// Versioned viewport event envelope.
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

/// JSON Lines direction marker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ViewportWireMessage {
    Command(ViewportCommandEnvelope),
    Event(ViewportEventEnvelope),
}
