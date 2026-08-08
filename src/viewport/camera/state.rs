//! Camera-owned viewport state.

use bevy::prelude::{Resource, Vec3};

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
