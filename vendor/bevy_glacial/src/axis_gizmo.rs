//! Visualization-only axis gizmo — a 3-arrow R/G/B triad drawn at an
//! entity's world transform. Pure debug overlay; no picking, no
//! interaction. Drawn each frame via Bevy's immediate-mode
//! [`Gizmos`] API (cheap — no entities, no meshes).
//!
//! Two ways to use it:
//!
//! 1. **Component-driven** (recommended): attach [`AxisGizmo`] to any
//!    entity with a [`GlobalTransform`] and add [`AxisGizmoPlugin`].
//!    The plugin will draw a triad at that entity each frame.
//!
//!    ```ignore
//!    commands.spawn((Transform::default(), AxisGizmo::default()));
//!    ```
//!
//! 2. **Direct call**: from your own system, call
//!    [`draw_axis_triad`] / [`draw_axis_triad_with_colors`] inside a
//!    `Gizmos`-borrowing system. Useful when you already iterate
//!    joints / bones / waypoints and want to draw without spawning
//!    a marker on each one (this is how `bevy_urdf`'s frame overlay
//!    works).

use bevy::prelude::*;

/// Default arrow colours: bright red / green / blue for X / Y / Z.
/// Same palette `bevy_urdf`'s overlays use, so visualisations stay
/// consistent across crates.
pub const DEFAULT_AXIS_COLORS: [Color; 3] = [
    Color::srgb(1.0, 0.1, 0.1),
    Color::srgb(0.1, 1.0, 0.1),
    Color::srgb(0.1, 0.3, 1.0),
];

/// Marker component — entities with this get an X / Y / Z arrow
/// triad drawn at their world transform each frame.
#[derive(Component, Clone, Copy, Debug)]
pub struct AxisGizmo {
    /// Arrow length in metres (each axis).
    pub length: f32,
    /// Per-axis colours, indexed `[X, Y, Z]`. Defaults to
    /// [`DEFAULT_AXIS_COLORS`].
    pub colors: [Color; 3],
}

impl Default for AxisGizmo {
    fn default() -> Self {
        Self {
            length: 1.0,
            colors: DEFAULT_AXIS_COLORS,
        }
    }
}

impl AxisGizmo {
    /// Convenience constructor: just a length, default R/G/B colours.
    pub fn new(length: f32) -> Self {
        Self {
            length,
            ..Default::default()
        }
    }
}

/// Draw a 3-axis arrow triad at `transform` with the default R/G/B
/// colours. Pure helper — call this from any system that already
/// borrows [`Gizmos`].
pub fn draw_axis_triad(gizmos: &mut Gizmos, transform: &GlobalTransform, length: f32) {
    draw_axis_triad_with_colors(gizmos, transform, length, &DEFAULT_AXIS_COLORS);
}

/// Same as [`draw_axis_triad`] with custom per-axis colours.
pub fn draw_axis_triad_with_colors(
    gizmos: &mut Gizmos,
    transform: &GlobalTransform,
    length: f32,
    colors: &[Color; 3],
) {
    let origin = transform.translation();
    let rotation = transform.rotation();
    let tip_x = origin + rotation * Vec3::X * length;
    let tip_y = origin + rotation * Vec3::Y * length;
    let tip_z = origin + rotation * Vec3::Z * length;
    gizmos.arrow(origin, tip_x, colors[0]);
    gizmos.arrow(origin, tip_y, colors[1]);
    gizmos.arrow(origin, tip_z, colors[2]);
}

/// Per-frame system: draw a triad at every entity that has both a
/// [`GlobalTransform`] and an [`AxisGizmo`] component.
pub fn draw_axis_gizmos(mut gizmos: Gizmos, targets: Query<(&GlobalTransform, &AxisGizmo)>) {
    for (gt, viz) in &targets {
        draw_axis_triad_with_colors(&mut gizmos, gt, viz.length, &viz.colors);
    }
}

/// Plugin: runs [`draw_axis_gizmos`] each frame. Harmless when no
/// entity carries [`AxisGizmo`].
pub struct AxisGizmoPlugin;

impl Plugin for AxisGizmoPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, draw_axis_gizmos);
    }
}
