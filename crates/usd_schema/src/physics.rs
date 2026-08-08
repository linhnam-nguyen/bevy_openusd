//! Decoded UsdPhysics records used by engine integrations.
//!
//! OpenUSD 0.5 exposes typed schema handles under `schemas::physics`; this
//! module provides the project-level aggregate readers consumed by the Bevy
//! and Rapier adapters.

use anyhow::Result;
use openusd::schemas::physics::tokens::*;
use openusd::sdf::{Path, Value};
use openusd::usd::{PrimPredicate, Stage};

pub use openusd::schemas::physics::{CollisionApprox, Dof, DriveType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JointKind {
    Fixed,
    Revolute,
    Prismatic,
    Spherical,
    Distance,
    Generic,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadRigidBody {
    pub rigid_body_enabled: bool,
    pub kinematic_enabled: bool,
    pub starts_asleep: bool,
    pub velocity: Option<[f32; 3]>,
    pub angular_velocity: Option<[f32; 3]>,
    pub simulation_owner: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadMass {
    pub mass: Option<f32>,
    pub center_of_mass: Option<[f32; 3]>,
    pub diagonal_inertia: Option<[f32; 3]>,
    pub principal_axes: Option<[f32; 4]>,
    pub density: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadPhysicsScene {
    pub path: String,
    pub gravity_direction: Option<[f32; 3]>,
    pub gravity_magnitude: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadCollisionShape {
    pub has_collision_api: bool,
    pub has_mesh_collision_api: bool,
    pub collision_enabled: bool,
    pub approximation: Option<CollisionApprox>,
    pub simulation_owner: Option<String>,
    pub physics_material_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadPhysicsMaterial {
    pub path: String,
    pub static_friction: Option<f32>,
    pub dynamic_friction: Option<f32>,
    pub restitution: Option<f32>,
    pub density: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReadLimit {
    pub dof: Dof,
    pub low: f32,
    pub high: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReadDrive {
    pub dof: Dof,
    pub drive_type: DriveType,
    pub target_position: Option<f32>,
    pub target_velocity: Option<f32>,
    pub damping: f32,
    pub stiffness: f32,
    pub max_force: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct ReadJoint {
    pub path: String,
    pub kind: JointKind,
    pub body0: Option<String>,
    pub body1: Option<String>,
    pub local_pos0: [f32; 3],
    pub local_rot0: [f32; 4],
    pub local_pos1: [f32; 3],
    pub local_rot1: [f32; 4],
    pub axis: Option<String>,
    pub lower_limit: Option<f32>,
    pub upper_limit: Option<f32>,
    pub collision_enabled: bool,
    pub joint_enabled: bool,
    pub exclude_from_articulation: bool,
    pub break_force: Option<f32>,
    pub break_torque: Option<f32>,
    pub min_distance: Option<f32>,
    pub max_distance: Option<f32>,
    pub cone_angle_0: Option<f32>,
    pub cone_angle_1: Option<f32>,
    pub limits: Vec<ReadLimit>,
    pub drives: Vec<ReadDrive>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadCollisionGroup {
    pub path: String,
    pub members: Vec<String>,
    pub filtered_groups: Vec<String>,
    pub merge_group: Option<String>,
    pub invert_filtered_groups: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadFilteredPairs {
    pub path: String,
    pub filtered: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PhysicsPrims {
    pub scenes: Vec<String>,
    pub rigid_bodies: Vec<String>,
    pub articulation_roots: Vec<String>,
    pub colliders: Vec<String>,
    pub joints: Vec<String>,
    pub materials: Vec<String>,
    pub collision_groups: Vec<String>,
    pub filtered_pairs: Vec<String>,
}

pub fn read_has_rigid_body(stage: &Stage, prim: &Path) -> Result<bool> {
    stage.prim(prim.clone()).has_api_schema(API_RIGID_BODY)
}

pub fn read_has_collision(stage: &Stage, prim: &Path) -> Result<bool> {
    stage.prim(prim.clone()).has_api_schema(API_COLLISION)
}

pub fn read_has_articulation_root(stage: &Stage, prim: &Path) -> Result<bool> {
    stage
        .prim(prim.clone())
        .has_api_schema(API_ARTICULATION_ROOT)
}

pub fn read_is_physics_scene(stage: &Stage, prim: &Path) -> Result<bool> {
    Ok(stage.prim(prim.clone()).type_name()?.as_deref() == Some(T_PHYSICS_SCENE))
}

pub fn read_rigid_body(stage: &Stage, prim: &Path) -> Result<Option<ReadRigidBody>> {
    if !read_has_rigid_body(stage, prim)? {
        return Ok(None);
    }
    Ok(Some(ReadRigidBody {
        rigid_body_enabled: read_bool(stage, prim, A_RIGID_BODY_ENABLED)?.unwrap_or(true),
        kinematic_enabled: read_bool(stage, prim, A_KINEMATIC_ENABLED)?.unwrap_or(false),
        starts_asleep: read_bool(stage, prim, A_STARTS_ASLEEP)?.unwrap_or(false),
        velocity: read_vec3f(stage, prim, A_VELOCITY)?,
        angular_velocity: read_vec3f(stage, prim, A_ANGULAR_VELOCITY)?,
        simulation_owner: read_rel_first_target(stage, prim, A_SIMULATION_OWNER)?,
    }))
}

pub fn read_mass(stage: &Stage, prim: &Path) -> Result<Option<ReadMass>> {
    if !stage.prim(prim.clone()).has_api_schema(API_MASS)? {
        return Ok(None);
    }
    Ok(Some(ReadMass {
        mass: read_scalar_f32(stage, prim, A_MASS)?,
        center_of_mass: read_vec3f(stage, prim, A_CENTER_OF_MASS)?,
        diagonal_inertia: read_vec3f(stage, prim, A_DIAGONAL_INERTIA)?,
        principal_axes: read_quatf(stage, prim, A_PRINCIPAL_AXES)?,
        density: read_scalar_f32(stage, prim, A_DENSITY)?,
    }))
}

pub fn read_physics_scene(stage: &Stage, prim: &Path) -> Result<Option<ReadPhysicsScene>> {
    if !read_is_physics_scene(stage, prim)? {
        return Ok(None);
    }
    Ok(Some(ReadPhysicsScene {
        path: prim.to_string(),
        gravity_direction: read_vec3f(stage, prim, A_GRAVITY_DIRECTION)?,
        gravity_magnitude: read_scalar_f32(stage, prim, A_GRAVITY_MAGNITUDE)?,
    }))
}

pub fn read_collision_shape(stage: &Stage, prim: &Path) -> Result<Option<ReadCollisionShape>> {
    let schemas = stage.prim(prim.clone()).api_schemas()?;
    if !schemas
        .iter()
        .any(|schema| schema.as_str() == API_COLLISION)
    {
        return Ok(None);
    }
    let has_mesh_collision_api = schemas
        .iter()
        .any(|schema| schema.as_str() == API_MESH_COLLISION);
    let approximation = if has_mesh_collision_api {
        read_token(stage, prim, A_APPROXIMATION)?.and_then(CollisionApprox::from_token)
    } else {
        None
    };
    Ok(Some(ReadCollisionShape {
        has_collision_api: true,
        has_mesh_collision_api,
        collision_enabled: read_bool(stage, prim, A_COLLISION_ENABLED)?.unwrap_or(true),
        approximation,
        simulation_owner: read_rel_first_target(stage, prim, A_SIMULATION_OWNER)?,
        physics_material_path: read_rel_first_target(stage, prim, REL_MATERIAL_BINDING_PHYSICS)?
            .or(read_rel_first_target(stage, prim, REL_MATERIAL_BINDING)?),
    }))
}

pub fn read_physics_material(stage: &Stage, prim: &Path) -> Result<Option<ReadPhysicsMaterial>> {
    if !stage
        .prim(prim.clone())
        .has_api_schema(API_PHYSICS_MATERIAL)?
    {
        return Ok(None);
    }
    Ok(Some(ReadPhysicsMaterial {
        path: prim.to_string(),
        static_friction: read_scalar_f32(stage, prim, A_STATIC_FRICTION)?,
        dynamic_friction: read_scalar_f32(stage, prim, A_DYNAMIC_FRICTION)?,
        restitution: read_scalar_f32(stage, prim, A_RESTITUTION)?,
        density: read_scalar_f32(stage, prim, A_DENSITY)?,
    }))
}

pub fn read_joint_limits(stage: &Stage, prim: &Path) -> Result<Vec<ReadLimit>> {
    let mut out = Vec::new();
    for schema in stage.prim(prim.clone()).api_schemas()? {
        let Some(dof_name) = schema
            .as_str()
            .strip_prefix(API_LIMIT)
            .and_then(|rest| rest.strip_prefix(':'))
        else {
            continue;
        };
        let Some(dof) = Dof::from_token(dof_name) else {
            continue;
        };
        let low = read_scalar_f32(
            stage,
            prim,
            &format!("limit:{dof_name}:physics:{LIMIT_SUB_LOW}"),
        )?
        .unwrap_or(0.0);
        let high = read_scalar_f32(
            stage,
            prim,
            &format!("limit:{dof_name}:physics:{LIMIT_SUB_HIGH}"),
        )?
        .unwrap_or(0.0);
        out.push(ReadLimit { dof, low, high });
    }
    Ok(out)
}

pub fn read_joint_drives(stage: &Stage, prim: &Path) -> Result<Vec<ReadDrive>> {
    let mut out = Vec::new();
    for schema in stage.prim(prim.clone()).api_schemas()? {
        let Some(dof_name) = schema
            .as_str()
            .strip_prefix(API_DRIVE)
            .and_then(|rest| rest.strip_prefix(':'))
        else {
            continue;
        };
        let Some(dof) = Dof::from_token(dof_name) else {
            continue;
        };
        let attr = |suffix: &str| format!("drive:{dof_name}:physics:{suffix}");
        let drive_type = read_token(stage, prim, &attr(DRIVE_SUB_TYPE))?
            .and_then(DriveType::from_token)
            .unwrap_or_default();
        out.push(ReadDrive {
            dof,
            drive_type,
            target_position: read_scalar_f32(stage, prim, &attr(DRIVE_SUB_TARGET_POSITION))?,
            target_velocity: read_scalar_f32(stage, prim, &attr(DRIVE_SUB_TARGET_VELOCITY))?,
            damping: read_scalar_f32(stage, prim, &attr(DRIVE_SUB_DAMPING))?.unwrap_or(0.0),
            stiffness: read_scalar_f32(stage, prim, &attr(DRIVE_SUB_STIFFNESS))?.unwrap_or(0.0),
            max_force: read_scalar_f32(stage, prim, &attr(DRIVE_SUB_MAX_FORCE))?,
        });
    }
    Ok(out)
}

pub fn read_joint(stage: &Stage, prim: &Path) -> Result<Option<ReadJoint>> {
    let kind = match stage
        .prim(prim.clone())
        .type_name()?
        .as_deref()
        .unwrap_or_default()
    {
        T_PHYSICS_FIXED_JOINT => JointKind::Fixed,
        T_PHYSICS_REVOLUTE_JOINT => JointKind::Revolute,
        T_PHYSICS_PRISMATIC_JOINT => JointKind::Prismatic,
        T_PHYSICS_SPHERICAL_JOINT => JointKind::Spherical,
        T_PHYSICS_DISTANCE_JOINT => JointKind::Distance,
        T_PHYSICS_JOINT => JointKind::Generic,
        _ => return Ok(None),
    };
    Ok(Some(ReadJoint {
        path: prim.to_string(),
        kind,
        body0: read_rel_first_target(stage, prim, A_BODY0)?,
        body1: read_rel_first_target(stage, prim, A_BODY1)?,
        local_pos0: read_vec3f(stage, prim, A_LOCAL_POS_0)?.unwrap_or([0.0; 3]),
        local_rot0: read_quatf(stage, prim, A_LOCAL_ROT_0)?.unwrap_or([1.0, 0.0, 0.0, 0.0]),
        local_pos1: read_vec3f(stage, prim, A_LOCAL_POS_1)?.unwrap_or([0.0; 3]),
        local_rot1: read_quatf(stage, prim, A_LOCAL_ROT_1)?.unwrap_or([1.0, 0.0, 0.0, 0.0]),
        axis: read_token(stage, prim, A_AXIS)?,
        lower_limit: read_scalar_f32(stage, prim, A_LOWER_LIMIT)?,
        upper_limit: read_scalar_f32(stage, prim, A_UPPER_LIMIT)?,
        collision_enabled: read_bool(stage, prim, A_JOINT_COLLISION_ENABLED)?.unwrap_or(false),
        joint_enabled: read_bool(stage, prim, A_JOINT_ENABLED)?.unwrap_or(true),
        exclude_from_articulation: read_bool(stage, prim, A_EXCLUDE_FROM_ARTICULATION)?
            .unwrap_or(false),
        break_force: read_scalar_f32(stage, prim, A_BREAK_FORCE)?,
        break_torque: read_scalar_f32(stage, prim, A_BREAK_TORQUE)?,
        min_distance: read_scalar_f32(stage, prim, A_MIN_DISTANCE)?,
        max_distance: read_scalar_f32(stage, prim, A_MAX_DISTANCE)?,
        cone_angle_0: read_scalar_f32(stage, prim, A_CONE_ANGLE_0_LIMIT)?,
        cone_angle_1: read_scalar_f32(stage, prim, A_CONE_ANGLE_1_LIMIT)?,
        limits: read_joint_limits(stage, prim)?,
        drives: read_joint_drives(stage, prim)?,
    }))
}

pub fn read_collision_group(stage: &Stage, prim: &Path) -> Result<Option<ReadCollisionGroup>> {
    if stage.prim(prim.clone()).type_name()?.as_deref() != Some(T_PHYSICS_COLLISION_GROUP) {
        return Ok(None);
    }
    Ok(Some(ReadCollisionGroup {
        path: prim.to_string(),
        members: read_rel_all_targets(stage, prim, "collection:colliders:includes")?,
        filtered_groups: read_rel_all_targets(stage, prim, A_FILTERED_GROUPS)?,
        merge_group: read_token(stage, prim, A_MERGE_GROUP)?,
        invert_filtered_groups: read_bool(stage, prim, A_INVERT_FILTERED_GROUPS)?.unwrap_or(false),
    }))
}

pub fn read_filtered_pairs(stage: &Stage, prim: &Path) -> Result<Option<ReadFilteredPairs>> {
    if !stage
        .prim(prim.clone())
        .has_api_schema(API_FILTERED_PAIRS)?
    {
        return Ok(None);
    }
    Ok(Some(ReadFilteredPairs {
        path: prim.to_string(),
        filtered: read_rel_all_targets(stage, prim, A_FILTERED_PAIRS)?,
    }))
}

pub fn find_physics_prims(stage: &Stage) -> Result<PhysicsPrims> {
    let mut out = PhysicsPrims::default();
    stage.traverse(PrimPredicate::DEFAULT, |path| {
        let prim = stage.prim(path.clone());
        if let Ok(Some(type_name)) = prim.type_name() {
            match type_name.as_str() {
                T_PHYSICS_SCENE => out.scenes.push(path.to_string()),
                T_PHYSICS_JOINT
                | T_PHYSICS_FIXED_JOINT
                | T_PHYSICS_REVOLUTE_JOINT
                | T_PHYSICS_PRISMATIC_JOINT
                | T_PHYSICS_SPHERICAL_JOINT
                | T_PHYSICS_DISTANCE_JOINT => out.joints.push(path.to_string()),
                T_PHYSICS_COLLISION_GROUP => out.collision_groups.push(path.to_string()),
                _ => {}
            }
        }
        if let Ok(schemas) = prim.api_schemas() {
            let has = |name| schemas.iter().any(|schema| schema.as_str() == name);
            let path = path.to_string();
            if has(API_RIGID_BODY) {
                out.rigid_bodies.push(path.clone());
            }
            if has(API_ARTICULATION_ROOT) {
                out.articulation_roots.push(path.clone());
            }
            if has(API_COLLISION) {
                out.colliders.push(path.clone());
            }
            if has(API_PHYSICS_MATERIAL) {
                out.materials.push(path.clone());
            }
            if has(API_FILTERED_PAIRS) {
                out.filtered_pairs.push(path);
            }
        }
    })?;
    Ok(out)
}

fn read_attr_value(stage: &Stage, prim: &Path, name: &str) -> Result<Option<Value>> {
    Ok(stage
        .attribute(prim.append_property(name)?)
        .get::<Value>()?)
}

fn read_bool(stage: &Stage, prim: &Path, name: &str) -> Result<Option<bool>> {
    Ok(match read_attr_value(stage, prim, name)? {
        Some(Value::Bool(value)) => Some(value),
        _ => None,
    })
}

fn read_scalar_f32(stage: &Stage, prim: &Path, name: &str) -> Result<Option<f32>> {
    Ok(match read_attr_value(stage, prim, name)? {
        Some(Value::Float(value)) => Some(value),
        Some(Value::Double(value)) => Some(value as f32),
        _ => None,
    })
}

fn read_vec3f(stage: &Stage, prim: &Path, name: &str) -> Result<Option<[f32; 3]>> {
    Ok(match read_attr_value(stage, prim, name)? {
        Some(Value::Vec3f(value)) => Some(value.into()),
        Some(Value::Vec3d(value)) => {
            let value: [f64; 3] = value.into();
            Some([value[0] as f32, value[1] as f32, value[2] as f32])
        }
        _ => None,
    })
}

fn read_quatf(stage: &Stage, prim: &Path, name: &str) -> Result<Option<[f32; 4]>> {
    Ok(match read_attr_value(stage, prim, name)? {
        Some(Value::Quatf(value)) => Some(value.into()),
        Some(Value::Quatd(value)) => {
            let value: [f64; 4] = value.into();
            Some([
                value[0] as f32,
                value[1] as f32,
                value[2] as f32,
                value[3] as f32,
            ])
        }
        _ => None,
    })
}

fn read_token(stage: &Stage, prim: &Path, name: &str) -> Result<Option<String>> {
    Ok(read_attr_value(stage, prim, name)?.and_then(crate::value_into_string))
}

fn read_rel_first_target(stage: &Stage, prim: &Path, name: &str) -> Result<Option<String>> {
    Ok(read_rel_all_targets(stage, prim, name)?.into_iter().next())
}

fn read_rel_all_targets(stage: &Stage, prim: &Path, name: &str) -> Result<Vec<String>> {
    Ok(stage
        .relationship(prim.append_property(name)?)
        .targets()?
        .into_iter()
        .map(|path| path.to_string())
        .collect())
}
