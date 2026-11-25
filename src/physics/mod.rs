//! Viewport-owned Rapier adapter for the current `usd_bevy` marker routes.
//!
//! `usd_bevy` projects authored physics opinions as Bevy components. This
//! module is the render-server host boundary: it reads those components and
//! delegates all USD-to-Rapier construction to the pure `usd_rapier` crate.
//! No legacy Bevy adapter or schema crate is involved.

mod systems;
mod types;

use bevy::prelude::*;

pub(crate) use types::{PhysicsActive, PhysicsWorld};

use systems::{
    convert_colliders, convert_joints, convert_rigid_bodies, physics_is_active, step_physics,
    sync_bodies_on_resume, writeback_transforms,
};

/// Installs the current `usd_bevy` → `usd_rapier` bridge and the writeback.
pub(crate) struct RapierPhysicsPlugin;

impl Plugin for RapierPhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PhysicsWorld>()
            .init_resource::<PhysicsActive>()
            .add_systems(
                Update,
                (
                    convert_rigid_bodies,
                    convert_colliders.after(convert_rigid_bodies),
                    convert_joints.after(convert_rigid_bodies),
                    step_physics.after(convert_colliders).after(convert_joints),
                )
                    .chain(),
            )
            .add_systems(PostUpdate, writeback_transforms.run_if(physics_is_active))
            .add_systems(Update, sync_bodies_on_resume.before(step_physics));
    }
}
