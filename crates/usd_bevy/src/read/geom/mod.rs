//! UsdGeom readers: Mesh, BasisCurves, NurbsCurves, NurbsPatch, TetMesh,
//! HermiteCurves, Points, PointInstancer, plus shape (Cube/Sphere/Cylinder),
//! purpose/visibility/kind, and custom-data introspection — all decoded from
//! the composed stage through openusd's public Prim / Attribute API.

mod curves;
mod custom;
mod mesh;
mod points;
mod shapes;
mod util;

pub use curves::{
    CurveBasis, CurveType, CurveWrap, ReadCurves, ReadHermiteCurves, ReadNurbsCurves,
    ReadNurbsPatch, read_curves, read_hermite_curves, read_nurbs_curves, read_nurbs_patch,
};
pub use custom::{
    CustomAttrValue, CustomDict, VisibilityState, read_asset_info, read_custom_attrs,
    read_custom_data, read_custom_layer_data, read_effective_purpose, read_kind, read_purpose,
    read_visibility,
};
pub use mesh::{
    Interpolation, MeshPrimvar, Orientation, ReadMesh, ReadSubset, SubdivScheme, read_mesh,
};
pub use points::{
    ReadPointInstancer, ReadPoints, ReadTetMesh, read_point_instancer, read_points, read_tetmesh,
};
pub use shapes::{
    Axis, ReadCylinder, read_capsule, read_cube_size, read_cylinder, read_double_attr,
    read_sphere_radius,
};
