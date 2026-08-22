//! Existing semantic viewport commands, events, and read models.
//!
//! These definitions are intentionally moved without changing their serde
//! representation. The legacy stdio adapter and the current UI therefore keep
//! their protocol-version-1 wire shape while the new session contract grows
//! around them.

mod commands;
mod constants;
mod editor;
mod events;
mod read_models;

pub use commands::{ViewportCommand, ViewportCommandEnvelope};
pub use constants::{
    DEFAULT_SCENE_PAGE_SIZE, DEFAULT_SCENE_SEARCH_PAGE_SIZE, MAX_EDITOR_TEXT_BYTES,
    MAX_RUNTIME_MUTATIONS, MAX_RUNTIME_SOURCE_ID_BYTES, MAX_SCENE_PAGE_SIZE,
    MAX_SCENE_SEARCH_RESULTS,
};
pub use editor::{
    EditorOperation, EditorPrimReadModel, EditorStateReadModel, EditorValue, RuntimeMutation,
    RuntimeMutationBatch,
};
pub use events::{ViewportEvent, ViewportEventEnvelope, ViewportWireMessage};
pub use read_models::{
    CameraSource, ColorRgb8, CurveTuning, FocusMode, GroundGridOrigin, OverlayKind,
    PresentationReadModel, PrimNodeReadModel, RenderMode, RendererConfiguration,
    SamplingPreference, SamplingProvider, SceneAnchor, SceneChildrenPage, ScenePageReference,
    SceneReadModel, SceneSearchMatch, SelectionPresentationSettings, SelectionReadModel,
    StageLoadState, StageReadModel, TimelineReadModel, ViewerEnvironmentSettings,
    ViewportReadModel,
};
