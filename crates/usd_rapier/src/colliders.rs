//! Build a Rapier `Collider` from authored UsdPhysics collision data.
//!
//! Approximation fallback table (PLAN.md §2.5):
//!
//! | Authored      | Static body | Dynamic body          |
//! | ------------- | ----------- | --------------------- |
//! | None/default  | TriMesh     | ConvexHull (warn)     |
//! | ConvexHull    | ConvexHull  | ConvexHull            |
//! | ConvexDecomp  | Decomp      | Decomp                |
//! | MeshSimplify  | TriMesh     | ConvexHull (warn)     |
//!
//! USD's `UsdGeomCylinder.axis` defaults to Z; Rapier's primitive
//! cylinder is along Y. Caller passes the authored axis vector via
//! `local_pose.rotation` already composed with the Y→axis remap, so
//! this module just hands the dims to Rapier.

use anyhow::Result;
use openusd::sdf::Path;
use openusd::usd::{PrimPredicate, Stage, included_paths};
use openusd_schemas::physics::{CollisionApprox, CollisionGroup, collision_group_table};
use rapier3d_f64::glamx::DVec3;
use rapier3d_f64::prelude::*;
use std::collections::HashMap;

/// Compute Rapier [`InteractionGroups`] (up to 32 bitplane groups) for every
/// collider on `stage` covered by [`openusd_schemas::physics::CollisionGroupTable`]
/// (`L41`, `F-C03-I1-03`).
///
/// Rejects stages with more than 32 distinct `PhysicsCollisionGroup` prims (which
/// exceed Rapier's 32-bit `Group` bitfield capacity) and validates the resulting
/// `InteractionGroups` against the native USD pairwise collision oracle across all
/// covered colliders so overlapping multi-group memberships never silently
/// approximate an unrepresentable pair matrix.
pub fn stage_collider_interaction_groups(stage: &Stage) -> Result<HashMap<Path, InteractionGroups>> {
    let table = collision_group_table(stage)?;
    let groups = table.groups();
    if groups.len() > 32 {
        anyhow::bail!(
            "USD collision groups ({}) exceed 32-bit Rapier InteractionGroups capacity",
            groups.len()
        );
    }
    let count = groups.len();
    let mut memberships: HashMap<Path, u32> = HashMap::new();
    let mut filters: HashMap<Path, u32> = HashMap::new();
    let mut collider_groups: HashMap<Path, Vec<usize>> = HashMap::new();

    for (group_index, group_path) in groups.iter().enumerate() {
        let Some(group) = CollisionGroup::get(stage, group_path.clone())? else {
            continue;
        };
        let query = group.colliders_collection().membership_query()?;
        let included = included_paths(stage, &query, PrimPredicate::DEFAULT_PROXIES)?;
        let member_bit = 1u32 << group_index;
        let mut filter_mask = 0u32;
        for other_index in 0..count {
            if table.collides_at(group_index, other_index) {
                filter_mask |= 1u32 << other_index;
            }
        }
        for collider_path in included {
            *memberships.entry(collider_path.clone()).or_default() |= member_bit;
            *filters.entry(collider_path.clone()).or_default() |= filter_mask;
            let list = collider_groups.entry(collider_path).or_default();
            if !list.contains(&group_index) {
                list.push(group_index);
            }
        }
    }

    let mut result = HashMap::with_capacity(memberships.len());
    for (path, membership_bits) in memberships {
        let filter_bits = filters.get(&path).copied().unwrap_or(u32::MAX);
        result.insert(
            path,
            InteractionGroups::new(
                Group::from_bits_retain(membership_bits),
                Group::from_bits_retain(filter_bits),
                InteractionTestMode::And,
            ),
        );
    }

    // Verify pairwise equivalence against the native USD CollisionGroupTable
    // oracle across all covered colliders (including self-pair and overlapping
    // multi-group memberships).
    let covered: Vec<(&Path, &InteractionGroups, &[usize])> = result
        .iter()
        .map(|(path, ig)| {
            let indices = collider_groups
                .get(path)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            (path, ig, indices)
        })
        .collect();
    for (i, (path_a, ig_a, groups_a)) in covered.iter().enumerate() {
        for (path_b, ig_b, groups_b) in &covered[i..] {
            let expected_usd = groups_a.iter().any(|&ga| {
                groups_b
                    .iter()
                    .any(|&gb| table.collides_at(ga, gb))
            });
            let actual_rapier = ig_a.test(**ig_b);
            if expected_usd != actual_rapier {
                anyhow::bail!(
                    "authored USD collision group pair matrix between {} and {} cannot be represented identically by 32-bit Rapier InteractionGroups (expected={expected_usd}, actual={actual_rapier})",
                    path_a,
                    path_b
                );
            }
        }
    }

    Ok(result)
}

/// Authored shape inputs. The mesh case carries vertices + optional
/// indices in the *body-local* frame (caller has already baked any
/// scale relative to the parent body and translated/rotated to the
/// collider's pose) — so this module never touches Bevy / scene-tree
/// info.
pub enum ShapeInput {
    Cube {
        size: f64,
    },
    Sphere {
        radius: f64,
    },
    Capsule {
        half: DVec3,
        radius: f64,
    },
    /// Cylinder authored along Y (caller composes Y→authored-axis
    /// rotation into `local_pose` before calling).
    Cylinder {
        half_height: f64,
        radius: f64,
    },
    /// Thin static ground / floor stand-in; UsdPhysics has no native
    /// plane shape, so we use a thin slab.
    Plane,
    Mesh {
        vertices: Vec<DVec3>,
        indices: Option<Vec<[u32; 3]>>,
        approx: Option<CollisionApprox>,
        is_dynamic: bool,
    },
}

/// Per-collider authored inputs. `local_pose` expresses the collider's
/// frame in the parent body's local coords (caller composes any
/// axis-remap quaternion). `user_data` flows to
/// `ColliderBuilder::user_data` (host's choice).
pub struct ColliderOpinion {
    pub shape: ShapeInput,
    pub local_pose: Pose,
    pub friction: Option<f64>,
    pub restitution: Option<f64>,
    pub collision_groups: Option<InteractionGroups>,
    pub user_data: u128,
}

/// Insert a collider into `colliders`, parented to `parent_body` if
/// provided. Returns the inserted handle, or `None` when the shape
/// can't be constructed (mesh with no usable vertices).
pub fn build_collider(
    colliders: &mut ColliderSet,
    bodies: &mut RigidBodySet,
    parent_body: Option<RigidBodyHandle>,
    op: ColliderOpinion,
) -> Result<Option<ColliderHandle>> {
    let builder_opt: Option<ColliderBuilder> = match op.shape {
        ShapeInput::Cube { size } => {
            let h = size * 0.5;
            Some(ColliderBuilder::cuboid(h, h, h))
        }
        ShapeInput::Sphere { radius } => Some(ColliderBuilder::ball(radius)),
        ShapeInput::Capsule { half, radius } => {
            Some(ColliderBuilder::capsule_from_endpoints(-half, half, radius))
        }
        ShapeInput::Cylinder {
            half_height,
            radius,
        } => Some(ColliderBuilder::cylinder(half_height, radius)),
        ShapeInput::Plane => Some(ColliderBuilder::cuboid(50.0, 0.001, 50.0)),
        ShapeInput::Mesh {
            vertices,
            indices,
            approx,
            is_dynamic,
        } => build_mesh_collider(vertices, indices, approx, is_dynamic),
    };
    let Some(mut builder) = builder_opt else {
        return Ok(None);
    };

    builder = builder.position(op.local_pose).user_data(op.user_data);
    if let Some(g) = op.collision_groups {
        builder = builder.collision_groups(g);
    }
    if let Some(f) = op.friction {
        builder = builder.friction(f);
    }
    if let Some(r) = op.restitution {
        builder = builder.restitution(r);
    }

    let collider = builder.build();
    let handle = if let Some(parent) = parent_body {
        colliders.insert_with_parent(collider, parent, bodies)
    } else {
        colliders.insert(collider)
    };
    Ok(Some(handle))
}

fn build_mesh_collider(
    vertices: Vec<DVec3>,
    indices: Option<Vec<[u32; 3]>>,
    approx: Option<CollisionApprox>,
    is_dynamic: bool,
) -> Option<ColliderBuilder> {
    let approx = approx.unwrap_or(CollisionApprox::None);
    match approx {
        CollisionApprox::ConvexHull => ColliderBuilder::convex_hull(&vertices),
        CollisionApprox::ConvexDecomposition => {
            let Some(idx) = indices else {
                log::warn!("usd_rapier: convex decomposition needs indexed mesh; skipping");
                return None;
            };
            Some(ColliderBuilder::convex_decomposition(&vertices, &idx))
        }
        CollisionApprox::None | CollisionApprox::MeshSimplification => {
            if is_dynamic {
                log::warn!(
                    "usd_rapier: mesh collider on dynamic body approx={approx:?}; \
                     falling back to ConvexHull (Rapier rejects trimesh dynamic)"
                );
                ColliderBuilder::convex_hull(&vertices)
            } else {
                let idx = indices.unwrap_or_else(|| {
                    (0..vertices.len() / 3)
                        .map(|i| [(i * 3) as u32, (i * 3 + 1) as u32, (i * 3 + 2) as u32])
                        .collect()
                });
                ColliderBuilder::trimesh(vertices, idx).ok()
            }
        }
        CollisionApprox::BoundingSphere | CollisionApprox::BoundingCube => {
            ColliderBuilder::convex_hull(&vertices)
        }
    }
}
