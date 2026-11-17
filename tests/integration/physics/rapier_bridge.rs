//! Current pure `usd_rapier` builder smoke test.

use rapier3d_f64::glamx::{DQuat, DVec3};
use rapier3d_f64::prelude::{ColliderSet, RigidBodySet};
use usd_rapier::bodies::{RigidBodyOpinion, build_rigid_body};
use usd_rapier::colliders::{ColliderOpinion, ShapeInput, build_collider};

#[test]
fn current_builders_insert_a_body_and_attached_collider() {
    let mut bodies = RigidBodySet::new();
    let mut colliders = ColliderSet::new();
    let body = build_rigid_body(
        &mut bodies,
        &RigidBodyOpinion {
            kinematic: false,
            enabled: true,
            starts_asleep: false,
            world_translation: DVec3::ZERO,
            world_rotation: DQuat::IDENTITY,
            linvel: DVec3::ZERO,
            angvel: DVec3::ZERO,
            mass: Some(1.0),
            center_of_mass: None,
            diagonal_inertia: None,
            principal_axes: None,
        },
        7,
    )
    .expect("body builds");
    let collider = build_collider(
        &mut colliders,
        &mut bodies,
        Some(body),
        ColliderOpinion {
            shape: ShapeInput::Cube { size: 1.0 },
            local_pose: rapier3d_f64::math::Pose {
                translation: DVec3::ZERO,
                rotation: DQuat::IDENTITY,
            },
            friction: None,
            restitution: None,
            collision_groups: None,
            user_data: 8,
        },
    )
    .expect("collider builds")
    .expect("cube produces a collider");

    assert!(bodies.get(body).is_some());
    assert!(colliders.get(collider).is_some());
}
