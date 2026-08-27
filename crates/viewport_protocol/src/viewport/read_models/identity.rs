use serde::{Deserialize, Serialize};

use crate::{ProtocolValidationError, SessionId};

/// Stable, renderer-neutral identity for a logical USD target.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SceneAnchor {
    pub session_id: Option<SessionId>,
    pub prim_path: String,
    pub instance_context: Option<String>,
}

impl SceneAnchor {
    pub fn active_session(prim_path: impl Into<String>) -> Self {
        Self {
            session_id: None,
            prim_path: prim_path.into(),
            instance_context: None,
        }
    }

    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        if self.prim_path.trim().is_empty() {
            return Err(ProtocolValidationError::EmptyField {
                field: "selection.target.prim_path",
            });
        }
        if !self.prim_path.starts_with('/') || self.prim_path.contains('\0') {
            return Err(ProtocolValidationError::InvalidInput {
                field: "selection.target.prim_path",
            });
        }
        if self
            .instance_context
            .as_deref()
            .is_some_and(|context| context.contains('\0'))
        {
            return Err(ProtocolValidationError::InvalidInput {
                field: "selection.target.instance_context",
            });
        }
        if let Some(session_id) = &self.session_id {
            session_id.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraSource {
    Arcball,
    Authored { prim_path: String },
}

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

/// Selects the reference plane used by the viewport ground grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundGridOrigin {
    /// Follow the lowest loaded renderable geometry bound.
    #[default]
    LoadedScene,
    /// Stay on the Bevy world-origin plane (`y = 0`).
    WorldOrigin,
}

/// Selects the renderer's primary visual representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderMode {
    /// Use one uniform material color for all rendered surfaces.
    UniformColor,
    #[default]
    Shaded,
    Wireframe,
    /// Use Bevy Solari when the negotiated renderer capabilities allow it.
    RayTraced,
}

/// Compact RGB color value that is safe to carry across the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorRgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl ColorRgb8 {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}
