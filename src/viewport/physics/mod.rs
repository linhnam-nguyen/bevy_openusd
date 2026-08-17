//! Viewport-owned Rapier adapter for the current `usd_bevy` marker routes.
//!
//! `usd_bevy` projects authored physics opinions as Bevy components. This
//! module is the render-server host boundary: it reads those components and
//! delegates all USD-to-Rapier construction to the pure `usd_rapier` crate.
//! No legacy Bevy adapter or schema crate is involved.

use std::collections::HashMap;

use bevy::math::Vec3 as BevyVec3;
use bevy::prelude::*;
use rapier3d_f64::glamx::{DQuat, DVec3};
use rapier3d_f64::prelude::*;
use usd_bevy::{UsdCollider, UsdJoint, UsdMass, UsdPhysicsJoint, UsdPrimRef, UsdRigidBody};
use usd_rapier::bodies::{RigidBodyOpinion, build_rigid_body};
use usd_rapier::colliders::{ColliderOpinion, ShapeInput, build_collider};
use usd_rapier::joints::{JointKind, ReadJoint, build_and_insert_joint};

/// Whether the viewport advances its Rapier world.
#[derive(Resource, Debug, Clone, Copy)]
pub(crate) struct PhysicsActive(pub(crate) bool);

impl Default for PhysicsActive {
    fn default() -> Self {
        Self(false)
    }
}

/// All Rapier state for the current live USD projection.
#[derive(Resource)]
pub(crate) struct PhysicsWorld {
    gravity: Vector,
    integration_parameters: IntegrationParameters,
    physics_pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad_phase: BroadPhaseBvh,
    narrow_phase: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd_solver: CCDSolver,
    entity_to_body: HashMap<Entity, RigidBodyHandle>,
    entity_to_collider: HashMap<Entity, ColliderHandle>,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        let mut integration_parameters = IntegrationParameters::default();
        integration_parameters.num_solver_iterations = 16;
        integration_parameters.num_internal_pgs_iterations = 4;

        let mut world = Self {
            gravity: Vector::new(0.0, -9.81, 0.0),
            integration_parameters,
            physics_pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: BroadPhaseBvh::new(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            entity_to_body: HashMap::new(),
            entity_to_collider: HashMap::new(),
        };

        // The visual ground is supplied by GroundGrid; this thin slab is the
        // corresponding physics surface and has no USD entity to write back.
        let _ = build_collider(
            &mut world.colliders,
            &mut world.bodies,
            None,
            ColliderOpinion {
                shape: ShapeInput::Plane,
                local_pose: Pose {
                    translation: DVec3::new(0.0, -0.5, 0.0),
                    rotation: DQuat::IDENTITY,
                },
                friction: Some(1.0),
                restitution: None,
                collision_groups: None,
                user_data: 0,
            },
        );
        world
    }
}

#[derive(Component)]
struct BodyAttached;

#[derive(Component)]
struct ColliderAttached;

#[derive(Component)]
struct JointAttached;

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

fn convert_rigid_bodies(
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

fn convert_colliders(
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

fn convert_joints(
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

fn step_physics(active: Res<PhysicsActive>, mut physics: ResMut<PhysicsWorld>) {
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

fn writeback_transforms(
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

fn sync_bodies_on_resume(
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

fn physics_is_active(active: Res<PhysicsActive>) -> bool {
    active.0
}

fn find_body_ancestor(
    child_of: Option<&ChildOf>,
    parents: &Query<&ChildOf>,
    bodies: &Query<(), With<BodyAttached>>,
) -> Option<Entity> {
    let mut parent = child_of.map(ChildOf::parent);
    while let Some(entity) = parent {
        if bodies.get(entity).is_ok() {
            return Some(entity);
        }
        parent = parents.get(entity).ok().map(ChildOf::parent);
    }
    None
}

fn local_pose(entity: Entity, body: Option<Entity>) -> Pose {
    if body == Some(entity) {
        return Pose {
            translation: DVec3::ZERO,
            rotation: DQuat::IDENTITY,
        };
    }
    // Current route markers do not carry authored collider-local poses. The
    // mesh/prim transform is already represented in the scene graph; use a
    // conservative zero local pose until a shape route exposes that field.
    let _ = entity;
    Pose {
        translation: DVec3::ZERO,
        rotation: DQuat::IDENTITY,
    }
}

fn mesh_shape(mesh: &Mesh) -> Option<ShapeInput> {
    let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION)?.as_float3()?;
    let vertices = positions
        .iter()
        .map(|p| DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64))
        .collect::<Vec<_>>();
    if vertices.is_empty() {
        return None;
    }
    let indices = mesh.indices().map(|indices| {
        indices
            .iter()
            .collect::<Vec<_>>()
            .chunks_exact(3)
            .map(|triangle| [triangle[0] as u32, triangle[1] as u32, triangle[2] as u32])
            .collect::<Vec<_>>()
    });
    Some(ShapeInput::Mesh {
        vertices,
        indices,
        approx: None,
        is_dynamic: true,
    })
}

fn joint_kind(kind: &str) -> JointKind {
    match kind.to_ascii_lowercase().as_str() {
        "fixed" => JointKind::Fixed,
        "revolute" => JointKind::Revolute,
        "prismatic" => JointKind::Prismatic,
        "spherical" => JointKind::Spherical,
        "distance" => JointKind::Distance,
        _ => JointKind::Generic,
    }
}

fn to_dvec3(value: BevyVec3) -> DVec3 {
    DVec3::new(value.x as f64, value.y as f64, value.z as f64)
}

fn to_dquat(value: Quat) -> DQuat {
    DQuat::from_xyzw(
        value.x as f64,
        value.y as f64,
        value.z as f64,
        value.w as f64,
    )
}
