//! # bevy_glacial — reusable 3D-scene niceties for Bevy editor apps.
//!
//! Project-agnostic Bevy primitives that pair well with
//! [`bevy_frost`](https://github.com/bresilla/bevy_frost): a
//! mouse-driven orbit camera, an optional follow rig, a self-fading
//! LOD ground grid, a selection-ring shader, and a transform-gizmo
//! (translate / rotate / scale handles) inlined from the upstream
//! [`transform-gizmo`](https://github.com/urholaukkarinen/transform-gizmo)
//! project by **Urho Laukkarinen** (MIT or Apache-2.0; full text at
//! `LICENSE-MIT.transform-gizmo` / `LICENSE-APACHE.transform-gizmo`).
//!
//! ## Shape
//!
//! Each feature is its own `Plugin` so apps can opt in to just what
//! they need:
//!
//! * [`ChaseCameraPlugin`](camera::ChaseCameraPlugin) — mouse pan /
//!   orbit / zoom for an entity tagged with [`ChaseCamera`].
//! * [`FollowCameraPlugin`](follow::FollowCameraPlugin) — optional
//!   [`FollowTarget`] component that steers a [`ChaseCamera`] at
//!   another entity with offset + smoothing.
//! * [`GroundGridPlugin`](grid::GroundGridPlugin) — auto-spawns the
//!   LOD ground grid and runs its follow / fade systems.
//! * [`SelectionRingPlugin`] — animated ring marker.
//! * [`AxisGizmoPlugin`](axis_gizmo::AxisGizmoPlugin) — visualization
//!   only: draws an R/G/B arrow triad at each entity tagged with
//!   [`AxisGizmo`]. No interaction.
//! * [`TransformGizmoPlugin`] — translate / rotate / scale handles.
//! * [`WindowSettingsPlugin`](window_settings::WindowSettingsPlugin)
//!   — persists primary-window size + position to
//!   `~/.config/<app>/window.txt`. Opt-in (needs an app name); not
//!   bundled in [`GlacialPlugins`].
//!
//! ## Getting started
//!
//! ```ignore
//! use bevy::prelude::*;
//! use bevy_glacial::prelude::*;
//!
//! fn main() {
//!     App::new()
//!         .add_plugins(DefaultPlugins)
//!         // Everything in one go:
//!         .add_plugins(GlacialPlugins)
//!         .run();
//! }
//! ```
//!
//! Or pick individual pieces — for example, gizmo only:
//!
//! ```ignore
//! App::new()
//!     .add_plugins(DefaultPlugins)
//!     .add_plugins(TransformGizmoPlugin)
//!     .run();
//! ```
//!
//! Or everything except the grid:
//!
//! ```ignore
//! use bevy::app::PluginGroup;
//!
//! App::new()
//!     .add_plugins(DefaultPlugins)
//!     .add_plugins(GlacialPlugins.build().disable::<GroundGridPlugin>())
//!     .run();
//! ```

pub mod axis_gizmo;
pub mod camera;
pub mod follow;
pub mod gizmo;
pub mod grid;
pub mod joint_gizmos;
pub mod prelude;
pub mod selection_ring;
pub mod window_settings;

pub use axis_gizmo::{
    AxisGizmo, AxisGizmoPlugin, DEFAULT_AXIS_COLORS, draw_axis_gizmos, draw_axis_triad,
    draw_axis_triad_with_colors,
};
pub use camera::{
    ChaseCamera, ChaseCameraPlugin, apply_rig, chase_camera_control, chase_camera_zoom,
    cursor_ray_to_ground,
};
pub use follow::{FollowCameraPlugin, FollowTarget, follow_camera_target};
pub use grid::{
    GridKind, GroundGrid, GroundGridPlugin, LEVEL_HALF, LEVEL_STEPS, LocalGrid, build_grid_meshes,
    spawn_circle_meshes, update_grid_alpha,
};
pub use joint_gizmos::{
    draw_cone_wireframe, draw_distance_envelope, draw_prismatic_limit_segment,
    draw_revolute_limit_arc,
};
pub use selection_ring::{
    SelectionRing, SelectionRingEntity, SelectionRingExtension, SelectionRingMaterial,
    SelectionRingPlugin, SelectionRingSettings,
};
pub use window_settings::{WindowGeometry, WindowSettingsPlugin};

pub use gizmo::{
    BoundsFace, BoundsGizmoTarget, DEFAULT_BOUNDS_MIN_THICKNESS, EnumSet, GizmoAutoScale,
    GizmoCamera, GizmoHotkeys, GizmoMode, GizmoOptions, GizmoOrientation, GizmoTarget,
    GizmoVisuals, TransformGizmoPlugin, auto_scale_gizmo_to_target, resize_transform_face,
};

use bevy::app::{PluginGroup, PluginGroupBuilder};

/// `PluginGroup` bundling every `bevy_glacial` plugin: camera, follow,
/// grid, selection-ring, transform-gizmo. Use `.disable::<T>()` on the
/// builder to drop any sub-plugin you don't want.
pub struct GlacialPlugins;

impl PluginGroup for GlacialPlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            .add(ChaseCameraPlugin)
            .add(FollowCameraPlugin)
            .add(GroundGridPlugin)
            .add(SelectionRingPlugin)
            .add(AxisGizmoPlugin)
            .add(TransformGizmoPlugin)
    }
}
