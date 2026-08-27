//! Existing semantic viewport commands, events, and read models.
//!
//! These definitions carry the versioned viewport wire contract. The richer
//! session contract grows around them without coupling transport to Bevy.

pub mod bim;
mod commands;
mod constants;
mod editor;
mod events;
mod read_models;

pub use bim::*;

pub use commands::{ViewportCommand, ViewportCommandEnvelope};
pub use constants::{
    DEFAULT_SCENE_PAGE_SIZE, DEFAULT_SCENE_SEARCH_PAGE_SIZE, MAX_EDITOR_TEXT_BYTES,
    MAX_RUNTIME_MUTATIONS, MAX_RUNTIME_SOURCE_ID_BYTES, MAX_SCENE_PAGE_SIZE,
    MAX_SCENE_SEARCH_RESULTS, MAX_SELECTION_TARGETS,
};
pub use editor::{
    EditorOperation, EditorPrimReadModel, EditorStateReadModel, EditorValue, RuntimeMutation,
    RuntimeMutationBatch,
};
pub use events::{ViewportEvent, ViewportEventEnvelope, ViewportWireMessage};
pub use read_models::{
    CameraOrientationReadModel, CameraSource, ColorRgb8, CurveTuning, DEFAULT_GIZMO_SIZE_LEVEL,
    FocusMode, GroundGridOrigin, MAX_GIZMO_SIZE_LEVEL, MIN_GIZMO_SIZE_LEVEL, OverlayKind,
    PresentationReadModel, PrimNodeReadModel, RenderMode, RendererConfiguration,
    SamplingPreference, SamplingProvider, SamplingReadModel, SceneAnchor, SceneChildrenPage,
    ScenePageReference, SceneReadModel, SceneSearchMatch, SectionBoxReadModel,
    SelectionPresentationSettings, SelectionReadModel, StageLoadState, StageReadModel,
    StandardView, TimelineReadModel, ViewerEnvironmentSettings, ViewerSettingsCapabilities,
    ViewerSettingsReadModel, ViewportReadModel,
};
