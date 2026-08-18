//! UI-neutral command, event, and read-model contracts for the USDHub viewport.
//!
//! The protocol is split by responsibility so transport adapters can depend on
//! stable wire types without depending on Bevy, egui, Frost, Tauri, or OpenUSD.
//! The legacy `Viewport*` types remain available for protocol-version-1 stdio
//! compatibility; the `Client*` and `Server*` types reserve the richer session
//! contract used by the remote viewport transport.

pub const PROTOCOL_VERSION: u16 = 1;

pub mod authorization;
pub mod capabilities;
pub mod codec;
pub mod commands;
pub mod delivery;
pub mod envelope;
pub mod events;
pub mod handshake;
pub mod input;
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
pub use stream::{
    ActiveStreamConfiguration, CodecId, StreamLimits, StreamStatistics, ViewportMetrics,
};
pub use viewport::{
    CameraSource, CurveTuning, DEFAULT_SCENE_PAGE_SIZE, DEFAULT_SCENE_SEARCH_PAGE_SIZE,
    EditorOperation, EditorPrimReadModel, EditorStateReadModel, EditorValue, FocusMode,
    GroundGridOrigin, MAX_EDITOR_TEXT_BYTES, MAX_RUNTIME_MUTATIONS, MAX_RUNTIME_SOURCE_ID_BYTES,
    MAX_SCENE_PAGE_SIZE, MAX_SCENE_SEARCH_RESULTS, OverlayKind, PresentationReadModel,
    PrimNodeReadModel, RuntimeMutation, RuntimeMutationBatch, SceneAnchor, SceneChildrenPage,
    ScenePageReference, SceneReadModel, SceneSearchMatch, SelectionReadModel, StageLoadState,
    StageReadModel, TimelineReadModel, ViewportCommand, ViewportCommandEnvelope, ViewportEvent,
    ViewportEventEnvelope, ViewportReadModel, ViewportWireMessage,
};
