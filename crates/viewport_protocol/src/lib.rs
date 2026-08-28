//! UI-neutral command, event, and read-model contracts for the USDHub viewport.
//!
//! The protocol is split by responsibility so transport adapters can depend on
//! stable wire types without depending on Bevy, egui, Frost, Tauri, or OpenUSD.
//! The `Viewport*` types carry the versioned viewport contract; the `Client*`
//! and `Server*` types reserve the richer session contract used by the remote
//! viewport transport.

pub const PROTOCOL_VERSION: u16 = 6;

pub use usd_model::{CanonicalValue, UnitId};

pub mod authorization;
pub mod capabilities;
pub mod codec;
pub mod commands;
pub mod delivery;
pub mod envelope;
pub mod events;
pub mod handshake;
pub mod input;
pub mod runtime_client;
pub mod semantic_sync;
pub mod stream;
pub mod viewport;

pub use authorization::{
    AuthorizationPolicy, AuthorizationValidationError, DeliveryMode, HistoryPermission,
    ModelDownloadPermission, RuntimeProfile, SemanticPropertyScope,
};
pub use capabilities::{ClientCapabilities, CommandFamily, InputCapabilities, ServerCapabilities};
pub use codec::{
    ClientWireMessage, ServerWireMessage, decode_client_json_line, decode_json_line,
    decode_server_json_line, encode_client_json_line, encode_json_line, encode_server_json_line,
};
pub use commands::{ClientCommand, InputCommand, SessionCommand, StreamCommand};
pub use delivery::{
    AuthorizedRuntimeManifest, RuntimeBlobReference, RuntimeManifest,
    RuntimeManifestAuthorizationError, RuntimeManifestValidationError, RuntimePayloadKind,
    validate_runtime_blob_id,
};
pub use envelope::{
    CausationId, ClientCommandEnvelope, ProtocolValidationError, RequestId, SequenceNumber,
    ServerEventEnvelope, SessionId, validate_protocol_version,
};
pub use events::{ServerEvent, SessionEvent, StreamEvent};
pub use handshake::{
    ClientHello, HandshakeEvent, HandshakeRejectionReason, ResumeRequest, ResumeResult,
    ServerHello, SessionRole,
};
pub use input::{
    ButtonState, FocusState, InputModifiers, KeyboardInput, PointerButtons, PointerMotion,
    ReleaseAllInput,
};
pub use runtime_client::{
    HydratedRuntimeDelivery, RuntimeDeliveryAssembler, RuntimeDeliveryClientError,
    RuntimeDeliveryUpdate,
};
pub use semantic_sync::{SemanticSyncOperation, SemanticSyncPhase, SemanticSyncStatus};
pub use stream::{
    ActiveStreamConfiguration, CodecId, StreamLimits, StreamStatistics, ViewportMetrics,
};
pub use viewport::{
    BimFieldKey, BimObjectMatch, BimPageRequest, BimPropertiesReadModel, BimPropertyDiffReadModel,
    BimPropertyDiffRow, BimPropertyDiffStatus, BimPropertyEditOutcome, BimPropertyEditStatus,
    BimPropertyGroupId, BimPropertyMutation, BimPropertyNameMatch, BimPropertyReadModel,
    BimPropertyValueMatch, BimReplacementPreviewRow, BimSearchQuery, BimSearchResult,
    BimUnitOption, CameraOrientationReadModel, CameraSource, ClassificationColorEntry,
    ClassificationColorIntent, ClassificationColorSource, ClassificationLevel,
    ClassificationRecipe, ColorRgb8, CommonValue, CurveTuning, DEFAULT_GIZMO_SIZE_LEVEL,
    DEFAULT_SCENE_PAGE_SIZE, DEFAULT_SCENE_SEARCH_PAGE_SIZE, EditorOperation, EditorPrimReadModel,
    EditorStateReadModel, EditorValue, FocusMode, GroundGridOrigin, HierarchyChildrenPage,
    HierarchyNodeId, HierarchyNodeKind, HierarchyNodeReadModel, HierarchyPageReference,
    HierarchyReadModel, HierarchySearchMatch, HierarchySource, MAX_BIM_BATCH_EDITS,
    MAX_BIM_CLASSIFICATION_LEVELS, MAX_BIM_CLASSIFICATION_PAGE_SIZE, MAX_BIM_FIELD_KEY_BYTES,
    MAX_BIM_REGEX_BYTES, MAX_BIM_REPLACEMENT_BYTES, MAX_BIM_SEARCH_GROUPS, MAX_BIM_SEARCH_OFFSET,
    MAX_BIM_SEARCH_PAGE_SIZE, MAX_BIM_SELECTION_TARGETS, MAX_CLASSIFICATION_COLOR_ENTRIES,
    MAX_EDITOR_TEXT_BYTES, MAX_GIZMO_SIZE_LEVEL, MAX_HIERARCHY_NODE_ID_BYTES,
    MAX_HIERARCHY_SEARCH_QUERY_BYTES, MAX_RUNTIME_MUTATIONS, MAX_RUNTIME_SOURCE_ID_BYTES,
    MAX_SCENE_PAGE_SIZE, MAX_SCENE_SEARCH_RESULTS, MAX_SELECTION_TARGETS, MIN_GIZMO_SIZE_LEVEL,
    OverlayKind, PresentationReadModel, PrimNodeReadModel, RenderMode, RendererConfiguration,
    RuntimeMutation, RuntimeMutationBatch, SamplingPreference, SamplingProvider, SamplingReadModel,
    SceneAnchor, SceneChildrenPage, ScenePageReference, SceneReadModel, SceneSearchMatch,
    SectionBoxReadModel, SelectionPresentationSettings, SelectionReadModel, StageLoadState,
    StageReadModel, StandardView, TimelineReadModel, UNCLASSIFIED_LABEL, ViewerEnvironmentSettings,
    ViewerSettingsCapabilities, ViewerSettingsReadModel, ViewportCommand, ViewportCommandEnvelope,
    ViewportEvent, ViewportEventEnvelope, ViewportReadModel, ViewportWireMessage, bim,
};
