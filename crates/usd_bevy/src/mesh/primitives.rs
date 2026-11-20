use crate::read::geom::{Axis, ReadCylinder};
use bevy::mesh::{Mesh, Meshable};
use bevy::prelude::{Capsule3d, Cuboid, Cylinder, Plane3d, Sphere};

/// Build a Bevy mesh from a UsdGeom.Cube's `size`. The USD cube is
/// size × size × size centred at the prim origin.
pub fn mesh_cube(size: f64) -> Mesh {
    Mesh::from(Cuboid::new(size as f32, size as f32, size as f32))
}

/// UsdGeom.Sphere radius → Bevy's UV sphere.
pub fn mesh_sphere(radius: f64) -> Mesh {
    Mesh::from(Sphere::new(radius as f32))
}

/// UsdGeom.Cylinder dimensions + axis. Bevy's `Cylinder` points up the Y
/// axis by convention, so we apply an axis rotation for X/Z cases.
pub fn mesh_cylinder(params: ReadCylinder) -> Mesh {
    let mut mesh = Mesh::from(Cylinder::new(params.radius as f32, params.height as f32));
    apply_axis(&mut mesh, params.axis);
    mesh
}

/// UsdGeom.Plane `width` × `length`. Y-normal plane centred at the origin.
pub fn mesh_plane(width: f64, length: f64) -> Mesh {
    Mesh::from(Plane3d::default().mesh().size(width as f32, length as f32))
}

/// UsdGeom.Capsule dimensions + axis. Bevy's `Capsule3d` is Y-axis aligned.
pub fn mesh_capsule(params: ReadCylinder) -> Mesh {
    // UsdGeom.Capsule's `height` is the cylinder portion length (hemispheres
    // add `2*radius` to the total). Bevy's Capsule3d takes `half_length` =
    // half the cylinder portion.
    let mut mesh = Mesh::from(Capsule3d::new(params.radius as f32, params.height as f32));
    apply_axis(&mut mesh, params.axis);
    mesh
}

/// Rotate vertices so a Y-up primitive faces the requested axis.
fn apply_axis(mesh: &mut Mesh, axis: Axis) {
    let rot = match axis {
        Axis::Y => return,
        Axis::X => bevy::math::Quat::from_rotation_z(-core::f32::consts::FRAC_PI_2),
        Axis::Z => bevy::math::Quat::from_rotation_x(core::f32::consts::FRAC_PI_2),
    };
    rotate_mesh(mesh, rot);
}

pub fn rotate_mesh(mesh: &mut Mesh, rot: bevy::math::Quat) {
    mesh.rotate_by(rot);
}
