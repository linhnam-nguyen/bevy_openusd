use serde::{Deserialize, Serialize};

use crate::{PROTOCOL_VERSION, RequestId};

use super::bim::BimPropertyEditOutcome;
use super::commands::ViewportCommandEnvelope;
use super::editor::{EditorOperation, EditorPrimReadModel, EditorStateReadModel};
use super::hierarchy::{HierarchyChildrenPage, HierarchySearchMatch, HierarchySource};
use super::read_models::{
    CameraOrientationReadModel, CameraSource, FocusMode, PresentationReadModel, SceneAnchor,
    SceneChildrenPage, SceneSearchMatch, SelectionReadModel, StageLoadState, TimelineReadModel,
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
    HierarchyChildren {
        source: HierarchySource,
        page: HierarchyChildrenPage,
    },
    HierarchySearchResults {
        source: HierarchySource,
        query: String,
        offset: u32,
        total: u32,
        matches: Vec<HierarchySearchMatch>,
        has_more: bool,
    },
    StageLoadStateChanged {
        state: StageLoadState,
    },
    SelectionChanged {
        selection: SelectionReadModel,
    },
    /// Applies one authoritative selection delta to the client's existing
    /// selection. Complete selection state remains available in snapshots.
    SelectionDeltaApplied {
        revision: u64,
        added: Vec<SceneAnchor>,
        removed: Vec<SceneAnchor>,
        primary: Option<SceneAnchor>,
        count: u32,
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
    CameraOrientationChanged {
        orientation: CameraOrientationReadModel,
    },
    CameraStandardViewStarted {
        view: super::read_models::StandardView,
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
    BimPropertyEditCompleted {
        outcome: BimPropertyEditOutcome,
        live_revision: u64,
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
