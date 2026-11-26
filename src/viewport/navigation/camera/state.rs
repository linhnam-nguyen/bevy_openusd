//! Camera-owned viewport state.

use bevy::prelude::{Quat, Resource, Vec3};
use viewport_protocol::CameraOrientationReadModel;

/// Renderer-owned ratios and safe empty-scene fallbacks for camera
/// navigation. User dolly has no scene-scale clamp; the finite values here
/// keep projection math valid across extreme CAD/BIM scales and when no
/// authored bounds are available.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct CameraNavigationConfig {
    /// Exponential wheel response applied to the browser/native wheel delta.
    pub zoom_speed: f64,
    /// Legacy scene-scale ratio retained for configuration compatibility;
    /// free-camera near planes derive from local inspection depth so large
    /// scenes never clip close-up detail.
    pub near_scene_scale_ratio: f64,
    /// Local near-plane ratio derived from the active camera-to-focus depth.
    pub near_focus_distance_ratio: f64,
    /// Extra far-plane coverage expressed as a ratio of the scene radius.
    pub far_safety_ratio: f64,
    /// Documented bounds used only while the active Scene is empty or has
    /// degenerate zero-size bounds.
    pub empty_scene_radius: f64,
    pub empty_scene_distance: f64,
}

impl Default for CameraNavigationConfig {
    fn default() -> Self {
        Self {
            zoom_speed: 0.01,
            near_scene_scale_ratio: 1.0e-5,
            near_focus_distance_ratio: 1.0e-3,
            far_safety_ratio: 0.1,
            empty_scene_radius: 1.0,
            empty_scene_distance: 4.0,
        }
    }
}

/// Retained world-space focus target re-evaluated against the active view
/// context during an in-flight `FlyTo` transition so mid-tween viewport
/// resizes, FOV changes, or projection switches never finish at a stale pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum FramingIntent {
    Bounds { min: Vec3, max: Vec3 },
    Point(Vec3),
}

/// An in-flight camera tween. `remaining` counts down by `delta_time` every
/// frame until zero, at which point the camera settles at the target.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct FlyTo {
    pub target_focus: Vec3,
    pub target_distance: f32,
    pub remaining: f32,
    pub duration: f32,
    pub start_focus: Vec3,
    pub start_distance: f32,
    pub start_yaw: Option<f32>,
    pub target_yaw: Option<f32>,
    pub start_elevation: Option<f32>,
    pub target_elevation: Option<f32>,
    pub start_ortho_scale: Option<f32>,
    pub target_ortho_scale: Option<f32>,
    pub(crate) framing_intent: Option<FramingIntent>,
}

impl FlyTo {
    /// Cancels any active transition and clears all optional angle,
    /// orthographic scale, and framing intent targets so superseded tweens
    /// cannot leak or resume.
    pub(crate) fn cancel(&mut self) {
        self.remaining = 0.0;
        self.duration = 0.0;
        self.start_yaw = None;
        self.target_yaw = None;
        self.start_elevation = None;
        self.target_elevation = None;
        self.start_ortho_scale = None;
        self.target_ortho_scale = None;
        self.framing_intent = None;
    }
}

/// Latest validated orientation sampled from the live viewport camera.
#[derive(Resource, Debug, Clone)]
pub(crate) struct CameraOrientationState {
    pub latest: CameraOrientationReadModel,
    pub(super) last_rotation: Option<Quat>,
    pub(super) last_published_at: f64,
    pub(super) has_published: bool,
}

impl Default for CameraOrientationState {
    fn default() -> Self {
        Self {
            latest: CameraOrientationReadModel::default(),
            last_rotation: None,
            last_published_at: 0.0,
            has_published: false,
        }
    }
}

/// Saved camera viewpoints. They are session-only until the future review
/// domain persists renderer-neutral viewpoints.
#[derive(Resource, Default, Debug, Clone)]
pub struct CameraBookmarks {
    pub items: Vec<CameraBookmark>,
    pub next_seq: u32,
}

#[derive(Debug, Clone)]
pub struct CameraBookmark {
    pub name: String,
    pub focus: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub elevation: f32,
}

/// Which camera drives the viewport.
#[derive(Resource, Debug, Clone, Default)]
pub enum CameraMount {
    #[default]
    Arcball,
    Mounted {
        /// Authored USD prim path of the mounted camera.
        prim_path: String,
    },
}
