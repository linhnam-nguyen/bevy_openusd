use openusd::sdf::Path;
use openusd::usd::Stage;

use super::util::{
    read_float_array, read_int_array, read_int64_array, read_quat_array, read_vec3f_array,
};

// ── PointInstancer ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct ReadPointInstancer {
    pub prototypes: Vec<Path>,
    pub positions: Vec<[f32; 3]>,
    pub orientations: Vec<[f32; 4]>,
    pub scales: Vec<[f32; 3]>,
    pub proto_indices: Vec<i32>,
    /// Authored logical IDs. `None` means the schema did not author `ids`,
    /// in which case the source row is the only available logical identity.
    pub ids: Option<Vec<i64>>,
}

pub fn read_point_instancer(
    stage: &Stage,
    prim: &Path,
) -> anyhow::Result<Option<ReadPointInstancer>> {
    let Some(positions) = read_vec3f_array(stage, prim, "positions")? else {
        return Ok(None);
    };
    let Some(proto_indices) = read_int_array(stage, prim, "protoIndices")? else {
        return Ok(None);
    };
    let prototypes = stage
        .prim(prim.clone())
        .relationship("prototypes")
        .targets()
        .unwrap_or_default();
    let orientations = read_quat_array(stage, prim, "orientations")?.unwrap_or_default();
    let scales = read_vec3f_array(stage, prim, "scales")?.unwrap_or_default();
    let ids = read_int64_array(stage, prim, "ids")?;
    Ok(Some(ReadPointInstancer {
        prototypes,
        positions,
        orientations,
        scales,
        proto_indices,
        ids,
    }))
}

// ── Points ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ReadPoints {
    pub points: Vec<[f32; 3]>,
    pub widths: Vec<f32>,
    pub display_color: Option<Vec<[f32; 3]>>,
    pub ids: Vec<i64>,
}

pub fn read_points(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadPoints>> {
    let Some(points) = read_vec3f_array(stage, prim, "points")? else {
        return Ok(None);
    };
    Ok(Some(ReadPoints {
        points,
        widths: read_float_array(stage, prim, "widths")?.unwrap_or_default(),
        display_color: read_vec3f_array(stage, prim, "primvars:displayColor")?,
        ids: read_int64_array(stage, prim, "ids")?.unwrap_or_default(),
    }))
}

// ── TetMesh ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ReadTetMesh {
    pub points: Vec<[f32; 3]>,
    pub tet_vertex_indices: Vec<i32>,
    pub surface_face_vertex_indices: Option<Vec<i32>>,
    pub display_color: Option<Vec<[f32; 3]>>,
}

pub fn read_tetmesh(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadTetMesh>> {
    let Some(points) = read_vec3f_array(stage, prim, "points")? else {
        return Ok(None);
    };
    let Some(tet_vertex_indices) = read_int_array(stage, prim, "tetVertexIndices")? else {
        return Ok(None);
    };
    Ok(Some(ReadTetMesh {
        points,
        tet_vertex_indices,
        surface_face_vertex_indices: read_int_array(stage, prim, "surfaceFaceVertexIndices")?,
        display_color: read_vec3f_array(stage, prim, "primvars:displayColor")?,
    }))
}
