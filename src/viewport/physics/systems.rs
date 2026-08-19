use std::collections::HashMap;

use bevy::math::Vec3 as BevyVec3;
use bevy::prelude::*;
use rapier3d_f64::glamx::{DQuat, DVec3};
use rapier3d_f64::prelude::*;
use usd_bevy::{UsdCollider, UsdJoint, UsdMass, UsdPhysicsJoint, UsdPrimRef, UsdRigidBody};
use usd_rapier::bodies::{RigidBodyOpinion, build_rigid_body};
use usd_rapier::colliders::{ColliderOpinion, ShapeInput, build_collider};
use usd_rapier::joints::{ReadJoint, build_and_insert_joint};

use super::types::{
    BodyAttached, ColliderAttached, JointAttached, PhysicsActive, PhysicsWorld, find_body_ancestor,
    joint_kind, local_pose, mesh_shape, to_dquat, to_dvec3,
};

pub(super) fn convert_rigid_bodies(
    mut commands: Commands,
    mut physics: ResMut<PhysicsWorld>,
    bodies: Query<
        (Entity, Option<&UsdMass>, Option<&GlobalTransform>),
        (With<UsdRigidBody>, Without<BodyAttached>),
    >,
) {
    for (entity, mass, global) in &bodies {
        let (translation, rotation) = global
            .map(|global| {
                let transform = global.compute_transform();
                (
                    to_dvec3(transform.translation),
                    to_dquat(transform.rotation),
                )
            })
            .unwrap_or((DVec3::ZERO, DQuat::IDENTITY));
        let opinion = RigidBodyOpinion {
            kinematic: false,
            enabled: true,
            starts_asleep: false,
            world_translation: translation,
            world_rotation: rotation,
            linvel: DVec3::ZERO,
            angvel: DVec3::ZERO,
            mass: mass.and_then(|mass| mass.mass).map(f64::from),
            center_of_mass: mass.and_then(|mass| mass.center_of_mass).map(to_dvec3),
            diagonal_inertia: mass.and_then(|mass| mass.diagonal_inertia).map(to_dvec3),
            principal_axes: None,
        };
        match build_rigid_body(&mut physics.bodies, &opinion, entity.to_bits() as u128) {
            Ok(handle) => {
                physics.entity_to_body.insert(entity, handle);
                commands.entity(entity).insert(BodyAttached);
            }
            Err(error) => warn!("usd_rapier: failed to build body {entity:?}: {error:#}"),
        }
    }
}

pub(super) fn convert_colliders(
    mut commands: Commands,
    mut physics: ResMut<PhysicsWorld>,
    colliders: Query<
        (
            Entity,
            Option<&UsdRigidBody>,
            Option<&Mesh3d>,
            Option<&ChildOf>,
        ),
        (With<UsdCollider>, Without<ColliderAttached>),
    >,
    bodies: Query<(), With<BodyAttached>>,
    parents: Query<&ChildOf>,
    meshes: Res<Assets<Mesh>>,
) {
    for (entity, rigid_body, mesh, child_of) in &colliders {
        let body_entity = rigid_body
            .map(|_| entity)
            .or_else(|| find_body_ancestor(child_of, &parents, &bodies));
        let parent_body = body_entity.and_then(|body| physics.entity_to_body.get(&body).copied());
        if body_entity.is_some() && parent_body.is_none() {
            continue;
        }

        let shape = mesh
            .and_then(|mesh| meshes.get(&mesh.0))
            .and_then(mesh_shape)
            .unwrap_or(ShapeInput::Cube { size: 1.0 });
        let local_pose = local_pose(entity, body_entity);
        let opinion = ColliderOpinion {
            shape,
            local_pose,
            friction: None,
            restitution: None,
            collision_groups: None,
            user_data: entity.to_bits() as u128,
        };
        let result = {
            let physics = &mut *physics;
            build_collider(
                &mut physics.colliders,
                &mut physics.bodies,
                parent_body,
                opinion,
            )
        };
        match result {
            Ok(Some(handle)) => {
                physics.entity_to_collider.insert(entity, handle);
                commands.entity(entity).insert(ColliderAttached);
            }
            Ok(None) => warn!("usd_rapier: no collider shape for {entity:?}"),
            Err(error) => warn!("usd_rapier: failed to build collider {entity:?}: {error:#}"),
        }
    }
}

pub(super) fn convert_joints(
    mut commands: Commands,
    mut physics: ResMut<PhysicsWorld>,
    joints: Query<(Entity, &UsdJoint), (With<UsdPhysicsJoint>, Without<JointAttached>)>,
    prims: Query<(Entity, &UsdPrimRef)>,
) {
    let entities_by_path: HashMap<&str, Entity> = prims
        .iter()
        .map(|(entity, prim)| (prim.path.as_str(), entity))
        .collect();
    for (entity, joint) in &joints {
        let Some(body0) = joint
            .body0
            .as_deref()
            .and_then(|path| entities_by_path.get(path))
            .and_then(|entity| physics.entity_to_body.get(entity).copied())
        else {
            continue;
        };
        let Some(body1) = joint
            .body1
            .as_deref()
            .and_then(|path| entities_by_path.get(path))
            .and_then(|entity| physics.entity_to_body.get(entity).copied())
        else {
            continue;
        };

        let read = ReadJoint {
            path: String::new(),
            kind: joint_kind(&joint.kind),
            body0: joint.body0.clone(),
            body1: joint.body1.clone(),
            local_pos0: [0.0; 3],
            local_rot0: [1.0, 0.0, 0.0, 0.0],
            local_pos1: [0.0; 3],
            local_rot1: [1.0, 0.0, 0.0, 0.0],
            axis: joint.axis.clone(),
            lower_limit: joint.lower,
            upper_limit: joint.upper,
            collision_enabled: true,
            joint_enabled: true,
            break_force: None,
            break_torque: None,
            exclude_from_articulation: false,
            limits: Vec::new(),
            drives: Vec::new(),
        };
        let result = {
            let physics = &mut *physics;
            build_and_insert_joint(
                &mut physics.multibody_joints,
                &mut physics.impulse_joints,
                &read,
                body0,
                body1,
                false,
            )
        };
        if let Err(error) = result {
            warn!("usd_rapier: failed to build joint {entity:?}: {error:#}");
            continue;
        }
        commands.entity(entity).insert(JointAttached);
    }
}

pub(super) fn step_physics(active: Res<PhysicsActive>, mut physics: ResMut<PhysicsWorld>) {
    if !active.0 {
        return;
    }
    let PhysicsWorld {
        gravity,
        integration_parameters,
        physics_pipeline,
        islands,
        broad_phase,
        narrow_phase,
        bodies,
        colliders,
        impulse_joints,
        multibody_joints,
        ccd_solver,
        ..
    } = &mut *physics;
    physics_pipeline.step(
        *gravity,
        integration_parameters,
        islands,
        broad_phase,
        narrow_phase,
        bodies,
        colliders,
        impulse_joints,
        multibody_joints,
        ccd_solver,
        &(),
        &(),
    );
}

pub(super) fn writeback_transforms(
    physics: Res<PhysicsWorld>,
    mut targets: Query<(Entity, &mut Transform, Option<&ChildOf>)>,
    parents: Query<&GlobalTransform>,
) {
    for (entity, mut transform, child_of) in &mut targets {
        let Some(handle) = physics.entity_to_body.get(&entity).copied() else {
            continue;
        };
        let Some(body) = physics.bodies.get(handle) else {
            continue;
        };
        let pose = body.position();
        let world_translation = BevyVec3::new(
            pose.translation.x as f32,
            pose.translation.y as f32,
            pose.translation.z as f32,
        );
        let world_rotation = Quat::from_xyzw(
            pose.rotation.x as f32,
            pose.rotation.y as f32,
            pose.rotation.z as f32,
            pose.rotation.w as f32,
        );
        if let Some(child_of) = child_of
            && let Ok(parent) = parents.get(child_of.parent())
        {
            let parent = parent.compute_transform();
            let inverse = parent.rotation.inverse();
            let delta = inverse * (world_translation - parent.translation);
            transform.translation = BevyVec3::new(
                delta.x / parent.scale.x,
                delta.y / parent.scale.y,
                delta.z / parent.scale.z,
            );
            transform.rotation = inverse * world_rotation;
        } else {
            transform.translation = world_translation;
            transform.rotation = world_rotation;
        }
    }
}

pub(super) fn sync_bodies_on_resume(
    active: Res<PhysicsActive>,
    mut previous: Local<bool>,
    mut physics: ResMut<PhysicsWorld>,
    transforms: Query<&GlobalTransform>,
) {
    let was_active = *previous;
    *previous = active.0;
    if !active.0 || was_active {
        return;
    }
    let pairs: Vec<_> = physics
        .entity_to_body
        .iter()
        .map(|(entity, handle)| (*entity, *handle))
        .collect();
    for (entity, handle) in pairs {
        let Ok(global) = transforms.get(entity) else {
            continue;
        };
        let transform = global.compute_transform();
        let Some(body) = physics.bodies.get_mut(handle) else {
            continue;
        };
        body.set_position(
            Pose {
                translation: to_dvec3(transform.translation),
                rotation: to_dquat(transform.rotation),
            },
            true,
        );
        body.set_linvel(DVec3::ZERO, true);
        body.set_angvel(DVec3::ZERO, true);
    }
}

pub(super) fn physics_is_active(active: Res<PhysicsActive>) -> bool {
    active.0
}
