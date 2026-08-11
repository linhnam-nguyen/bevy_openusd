//! UI-neutral command, event, and read-model contracts for the USDHub viewport.
//!
//! The protocol is split by responsibility so transport adapters can depend on
//! stable wire types without depending on Bevy, egui, Frost, Tauri, or OpenUSD.
//! The legacy `Viewport*` types remain available for protocol-version-1 stdio
//! compatibility; the `Client*` and `Server*` types reserve the richer session
//! contract used by the remote viewport transport.

pub const PROTOCOL_VERSION: u16 = 1;

pub mod capabilities;
pub mod codec;
pub mod commands;
pub mod envelope;
pub mod events;
pub mod handshake;
pub mod input;
pub mod stream;
pub mod viewport;

pub use capabilities::{
    ClientCapabilities, CommandFamily, InputCapabilities, ServerCapabilities,
};
pub use codec::{
    ClientWireMessage, ServerWireMessage, decode_client_json_line, decode_json_line,
    decode_server_json_line, encode_client_json_line, encode_json_line, encode_server_json_line,
};
pub use commands::{
    ClientCommand, InputCommand, SessionCommand, StreamCommand,
};
pub use envelope::{
    validate_protocol_version, CausationId, ClientCommandEnvelope, ProtocolValidationError,
    RequestId, SequenceNumber, ServerEventEnvelope, SessionId,
};
pub use events::{ServerEvent, SessionEvent, StreamEvent};
pub use handshake::{
    ClientHello, HandshakeEvent, HandshakeRejectionReason, ResumeRequest, ResumeResult, ServerHello,
    SessionRole,
};
pub use input::{
    ButtonState, FocusState, InputModifiers, KeyboardInput, PointerButtons, PointerMotion,
    ReleaseAllInput,
};
pub use stream::{
    ActiveStreamConfiguration, CodecId, StreamLimits, StreamStatistics, ViewportMetrics,
};
pub use viewport::{
    CameraSource, CurveTuning, FocusMode, OverlayKind, PrimNodeReadModel, PresentationReadModel,
    SceneAnchor, SceneReadModel, SelectionReadModel, StageLoadState, StageReadModel,
    TimelineReadModel, ViewportCommand, ViewportEvent, ViewportEventEnvelope, ViewportReadModel,
    ViewportWireMessage, ViewportCommandEnvelope,
};
