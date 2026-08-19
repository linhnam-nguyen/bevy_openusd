use std::collections::HashMap;

use bevy::math::Vec3 as BevyVec3;
use bevy::prelude::*;
use rapier3d_f64::glamx::{DQuat, DVec3};
use rapier3d_f64::prelude::*;
use usd_rapier::colliders::{ColliderOpinion, ShapeInput, build_collider};
use usd_rapier::joints::JointKind;

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
    pub(super) gravity: Vector,
    pub(super) integration_parameters: IntegrationParameters,
    pub(super) physics_pipeline: PhysicsPipeline,
    pub(super) islands: IslandManager,
    pub(super) broad_phase: BroadPhaseBvh,
    pub(super) narrow_phase: NarrowPhase,
    pub(super) bodies: RigidBodySet,
    pub(super) colliders: ColliderSet,
    pub(super) impulse_joints: ImpulseJointSet,
    pub(super) multibody_joints: MultibodyJointSet,
    pub(super) ccd_solver: CCDSolver,
    pub(super) entity_to_body: HashMap<Entity, RigidBodyHandle>,
    pub(super) entity_to_collider: HashMap<Entity, ColliderHandle>,
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
pub(super) struct BodyAttached;

#[derive(Component)]
pub(super) struct ColliderAttached;

#[derive(Component)]
pub(super) struct JointAttached;

pub(super) fn find_body_ancestor(
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

pub(super) fn local_pose(entity: Entity, body: Option<Entity>) -> Pose {
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

pub(super) fn mesh_shape(mesh: &Mesh) -> Option<ShapeInput> {
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

pub(super) fn joint_kind(kind: &str) -> JointKind {
    match kind.to_ascii_lowercase().as_str() {
        "fixed" => JointKind::Fixed,
        "revolute" => JointKind::Revolute,
        "prismatic" => JointKind::Prismatic,
        "spherical" => JointKind::Spherical,
        "distance" => JointKind::Distance,
        _ => JointKind::Generic,
    }
}

pub(super) fn to_dvec3(value: BevyVec3) -> DVec3 {
    DVec3::new(value.x as f64, value.y as f64, value.z as f64)
}

pub(super) fn to_dquat(value: Quat) -> DQuat {
    DQuat::from_xyzw(
        value.x as f64,
        value.y as f64,
        value.z as f64,
        value.w as f64,
    )
}
