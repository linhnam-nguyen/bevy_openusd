use serde::{Deserialize, Serialize};

use super::identity::{ColorRgb8, RenderMode};
use crate::ProtocolValidationError;
use crate::stream::{MAX_FPS, MIN_FPS};

/// Viewer environment fields not already owned by the renderer presentation
/// read model. Renderer mode, shadows, grid visibility, and grid origin remain
/// authoritative in [`RendererConfiguration`] and [`PresentationReadModel`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerEnvironmentSettings {
    pub grid_color: ColorRgb8,
    pub background_color: ColorRgb8,
    pub default_surface_color: ColorRgb8,
}

impl Default for ViewerEnvironmentSettings {
    fn default() -> Self {
        Self {
            grid_color: ColorRgb8::new(0x6B, 0x72, 0x80),
            background_color: ColorRgb8::new(0x11, 0x18, 0x27),
            default_surface_color: ColorRgb8::new(0x9C, 0xA3, 0xAF),
        }
    }
}

/// Renderer-neutral selection presentation preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionPresentationSettings {
    pub boundary_enabled: bool,
    pub boundary_color: ColorRgb8,
    pub color_change_enabled: bool,
    pub selection_color: ColorRgb8,
    pub hover_color_change_enabled: bool,
    pub hover_color: ColorRgb8,
}

impl Default for SelectionPresentationSettings {
    fn default() -> Self {
        Self {
            boundary_enabled: true,
            boundary_color: ColorRgb8::new(0xFA, 0xCC, 0x15),
            color_change_enabled: false,
            selection_color: ColorRgb8::new(0x38, 0xBD, 0xF8),
            hover_color_change_enabled: false,
            hover_color: ColorRgb8::new(0x7D, 0xD3, 0xFC),
        }
    }
}

/// Vendor-neutral sampling intent. The active provider is authoritative
/// read-only state and is represented separately by [`SamplingProvider`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SamplingPreference {
    pub enabled: bool,
}

/// Renderer-selected sampling provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplingProvider {
    #[default]
    None,
    Dlss,
    Fsr,
}

/// Authoritative sampling state. The preference is user intent; the provider
/// is selected by the server and is never supplied by a client command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SamplingReadModel {
    pub preference: SamplingPreference,
    pub provider: SamplingProvider,
}

/// The one aggregate Section Box follows the complete authoritative selection
/// set. Plane transforms and applied clipping are intentionally deferred to
/// B6; the selection read model remains the target-set authority.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SectionBoxReadModel {
    pub enabled: bool,
}

/// Capability flags for settings that require renderer/device integrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ViewerSettingsCapabilities {
    pub ray_traced_supported: bool,
    pub dlss_available: bool,
    pub fsr_available: bool,
}

/// Applied settings exposed to reconnecting clients. This is protocol state,
/// not a claim that every renderer integration has already been implemented.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ViewerSettingsReadModel {
    pub environment: ViewerEnvironmentSettings,
    pub sampling: SamplingReadModel,
    pub selection: SelectionPresentationSettings,
    pub section_box: SectionBoxReadModel,
    pub capabilities: ViewerSettingsCapabilities,
}

/// Transport-neutral renderer options shared by commands and future
/// authoritative presentation events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RendererConfiguration {
    pub grid: bool,
    pub shadows: bool,
    pub edges: bool,
    pub render_mode: RenderMode,
    pub preferred_fps: Option<u32>,
}

impl Default for RendererConfiguration {
    fn default() -> Self {
        Self {
            grid: true,
            shadows: true,
            edges: false,
            render_mode: RenderMode::Shaded,
            preferred_fps: Some(60),
        }
    }
}

impl RendererConfiguration {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        if matches!(
            self.render_mode,
            RenderMode::UniformColor | RenderMode::RayTraced
        ) {
            return Err(ProtocolValidationError::InvalidInput {
                field: "renderer.render_mode",
            });
        }
        if let Some(fps) = self.preferred_fps
            && !(MIN_FPS..=MAX_FPS).contains(&fps)
        {
            return Err(ProtocolValidationError::InvalidFrameRate { value: fps });
        }
        Ok(())
    }
}
