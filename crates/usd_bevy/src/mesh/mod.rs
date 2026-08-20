//! UsdGeom → `bevy::render::mesh::Mesh`.
//!
//! Two kinds of input:
//! - Full meshes (`UsdGeom.Mesh`) — converts points / face indices / normals
//!   / uvs, fan-triangulates faces > 3 verts, expands `faceVarying` primvars.
//! - Primitive shapes (`Cube`, `Sphere`, `Cylinder`, `Capsule`) — delegate
//!   to Bevy's built-in `Meshable` primitives with the right dimensions.
//!
//! Orientation (`"leftHanded"` flips winding) and missing-normal fallback
//! (`compute_smooth_normals`) are handled here.

mod builder;
mod normals;
mod primitives;
mod primvar;
mod triangulation;

pub use builder::{mesh_from_usd, mesh_from_usd_subset};
pub use primitives::{
    mesh_capsule, mesh_cube, mesh_cylinder, mesh_plane, mesh_sphere, rotate_mesh,
};
