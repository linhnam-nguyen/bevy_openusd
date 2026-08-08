//! Physics-specific runtime glue for the native viewport.

use bevy::prelude::*;
use bevy::scene::ScenePatchInstance;

use crate::viewport::scene::visualization::{DisplayToggles, SceneExtent};

/// One-shot: after the scene has had a few ticks to populate (so
/// physics colliders exist and `SceneExtent` reports the asset's true
/// world bounds), shift the scene root up so its lowest point sits a
/// hair above the ground plane (Y=0). Without this, robotics assets
/// authored with their reference frame at the chassis centre spawn
/// with wheels deep inside the ground; the contact solver then
/// launches the chassis on tick 0.
/// Raises spawned scene roots when needed to rest their bounds on the physics ground.
pub(crate) fn lift_scene_off_ground(
    extent: Res<SceneExtent>,
    physics_active: Res<usd_bevy::physics::PhysicsActive>,
    mut scene_roots: Query<&mut Transform, With<ScenePatchInstance>>,
    mut done: Local<bool>,
) {
    if *done || extent.count == 0 {
        return;
    }
    let force_lift = std::env::var("BEVY_OPENUSD_AUTO_LIFT")
        .ok()
        .map(|v| matches!(v.as_str(), "1" | "true" | "on"))
        .unwrap_or(false);
    if !physics_active.0 && !force_lift {
        *done = true;
        return;
    }
    // Wait until the bbox has settled (more than a couple prims), so
    // we don't lift based on the partial extent of the first few
    // arrived prims.
    if extent.count < 8 {
        return;
    }
    let lowest = extent.min.y;
    let target_clearance = 0.01_f32;
    if lowest < target_clearance {
        let lift = target_clearance - lowest;
        for mut t in scene_roots.iter_mut() {
            t.translation.y += lift;
        }
        info!(
            "physics: lifted scene by {lift:.3} m so lowest extent y={lowest:.3} clears the ground"
        );
    }
    *done = true;
}

/// Wire `DisplayToggles.show_colliders` to the adapter's gizmo
/// renderer (`ColliderDebugEnabled` resource).
/// Enables Rapier collider debug rendering only when the user requests it.
pub(crate) fn sync_collider_debug_visibility(
    toggles: Res<DisplayToggles>,
    mut enabled: ResMut<usd_bevy::physics::ColliderDebugEnabled>,
) {
    if enabled.0 != toggles.show_colliders {
        enabled.0 = toggles.show_colliders;
    }
}

/// Static ground for the loaded scene — a cuboid in the adapter's
/// PhysicsWorld. Built directly into Rapier (not as a Bevy entity)
/// since it needs no Transform writeback. The visual floor comes
/// from glacial's `GroundGridPlugin` rendering an infinite-fade grid.
/// Adds the viewer's static floor collider to the USD physics world.
pub(crate) fn spawn_physics_ground(mut world: ResMut<usd_bevy::physics::PhysicsWorld>) {
    use rapier3d_f64::glamx::DVec3;
    use rapier3d_f64::prelude::*;
    let ground = ColliderBuilder::cuboid(50.0, 0.5, 50.0)
        .translation(DVec3::new(0.0, -0.5, 0.0))
        .friction(1.0)
        .build();
    world.colliders.insert(ground);
}
