//! The common import line for apps building on top of `bevy_glacial`.
//!
//! ```ignore
//! use bevy::prelude::*;
//! use bevy_glacial::prelude::*;
//! ```

pub use crate::joint_gizmos::{
    draw_cone_wireframe, draw_distance_envelope, draw_prismatic_limit_segment,
    draw_revolute_limit_arc,
};
pub use crate::{
    GlacialPlugins,
    axis_gizmo::{
        AxisGizmo, AxisGizmoPlugin, DEFAULT_AXIS_COLORS, draw_axis_gizmos, draw_axis_triad,
        draw_axis_triad_with_colors,
    },
    camera::{
        ChaseCamera, ChaseCameraPlugin, apply_rig, chase_camera_control, chase_camera_zoom,
        cursor_ray_to_ground,
    },
    follow::{FollowCameraPlugin, FollowTarget, follow_camera_target},
    grid::{
        GlacialGridCounters, GridKind, GroundGrid, GroundGridPlugin, LEVEL_HALF, LEVEL_STEPS,
        LocalGrid, build_grid_meshes, spawn_circle_meshes, update_grid_alpha,
    },
    selection_ring::{
        SelectionRing, SelectionRingEntity, SelectionRingExtension, SelectionRingMaterial,
        SelectionRingPlugin, SelectionRingSettings,
    },
    window_settings::{WindowGeometry, WindowSettingsPlugin},
};

// Vendored transform-gizmo passthrough.
pub use crate::gizmo::{
    BoundsFace, BoundsGizmoTarget, DEFAULT_BOUNDS_MIN_THICKNESS, EnumSet, GizmoAutoScale,
    GizmoCamera, GizmoHotkeys, GizmoMode, GizmoOptions, GizmoOrientation, GizmoTarget,
    GizmoVisuals, TransformGizmoPlugin, auto_scale_gizmo_to_target, resize_transform_face,
};
