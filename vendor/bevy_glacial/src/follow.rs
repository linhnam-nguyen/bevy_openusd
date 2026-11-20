//! Optional camera-follow rig — a `FollowTarget` component that
//! steers a sibling [`ChaseCamera`]'s focus point at another entity
//! with a configurable world-space offset and exponential smoothing.
//!
//! Attach [`FollowTarget`] to the same entity that has [`ChaseCamera`]
//! (typically your `Camera3d`). The follow system runs in `Update`
//! before [`chase_camera_control`], so the camera's pose is recomputed
//! from the freshly-written focus on the same frame.
//!
//! ```ignore
//! commands.spawn((
//!     Camera3d::default(),
//!     ChaseCamera::default(),
//!     FollowTarget {
//!         target: vehicle_entity,
//!         offset: Vec3::new(0.0, 1.5, 0.0),
//!         lerp_speed: 8.0,
//!     },
//! ));
//! ```
//!
//! Mouse panning still works while a target is set — the next frame
//! the follow system overwrites the focus, which is usually what you
//! want. To temporarily release the camera, remove or null the
//! [`FollowTarget`].

use bevy::prelude::*;

use crate::camera::{ChaseCamera, chase_camera_control};

/// Make the camera entity track another entity's position.
///
/// `lerp_speed` is an exponential rate (1/seconds): higher = the
/// focus catches up faster. Set it to `0.0` to snap with no
/// smoothing. The default (`8.0`) reaches ~98 % of the target in
/// half a second.
#[derive(Component, Clone, Copy, Debug)]
pub struct FollowTarget {
    /// Entity to follow. Must have a [`GlobalTransform`].
    pub target: Entity,
    /// World-space offset added to the target's translation before
    /// it becomes the camera's focus.
    pub offset: Vec3,
    /// Exponential smoothing rate (1/seconds). `0.0` = snap.
    pub lerp_speed: f32,
}

impl Default for FollowTarget {
    fn default() -> Self {
        Self {
            target: Entity::PLACEHOLDER,
            offset: Vec3::ZERO,
            lerp_speed: 8.0,
        }
    }
}

/// Per-frame system: write each [`ChaseCamera`]'s focus from its
/// [`FollowTarget`]'s world position + offset, with exponential
/// smoothing.
pub fn follow_camera_target(
    time: Res<Time>,
    targets: Query<&GlobalTransform>,
    mut cameras: Query<(&FollowTarget, &mut ChaseCamera)>,
) {
    let dt = time.delta_secs();
    for (follow, mut cam) in &mut cameras {
        let Ok(target_tr) = targets.get(follow.target) else {
            continue;
        };
        let desired = target_tr.translation() + follow.offset;
        if follow.lerp_speed <= 0.0 {
            cam.focus = desired;
        } else {
            // Time-step-independent exponential smoothing: alpha
            // = 1 - exp(-k·dt). At k=8, dt=0.5s ⇒ alpha ≈ 0.98.
            let alpha = (1.0 - (-follow.lerp_speed * dt).exp()).clamp(0.0, 1.0);
            cam.focus = cam.focus.lerp(desired, alpha);
        }
    }
}

/// Plugin: runs [`follow_camera_target`] before
/// [`chase_camera_control`] each `Update`. Harmless when no entity
/// has [`FollowTarget`].
pub struct FollowCameraPlugin;

impl Plugin for FollowCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, follow_camera_target.before(chase_camera_control));
    }
}
