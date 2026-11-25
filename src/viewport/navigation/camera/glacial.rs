//! Adapter between the viewport's arcball camera and Glacial's grid camera.

use bevy::prelude::*;
use bevy_glacial::prelude::ChaseCamera;

use super::ArcballCamera;

/// Mirrors arcball focus and distance into the LOD ground-grid camera.
pub(crate) fn sync_chase_camera(mut q: Query<(&ArcballCamera, &mut ChaseCamera)>) {
    for (arc, mut chase) in q.iter_mut() {
        chase.focus = arc.focus;
        chase.distance = arc.distance;
        chase.yaw = arc.yaw;
        chase.elevation = arc.elevation;
    }
}
