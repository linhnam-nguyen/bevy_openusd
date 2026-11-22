use openusd::sdf::Path;
use openusd::usd::{Stage, TimeCode};
use openusd::vt::Value;

use super::util::{read_float_array, read_int_array, read_int64_array, read_vec3f_array};

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
    read_point_instancer_at(stage, prim, None)
}

/// Sample the point-instancer arrays together at one stage time.
pub fn read_point_instancer_at(
    stage: &Stage,
    prim: &Path,
    time: Option<f64>,
) -> anyhow::Result<Option<ReadPointInstancer>> {
    let owner = stage.prim(prim.clone())?;
    let value = |name| {
        let attribute = owner.attribute(name);
        let default = || {
            let value = attribute.get::<Value>()?;
            if value
                .as_ref()
                .is_some_and(|value| value.array_len() != Some(0))
            {
                return Ok(value);
            }
            // Some caches author an empty default as a placeholder. Preserve
            // the original first-sample fallback, while allowing an empty
            // table with no samples to remove all rows.
            Ok(attribute
                .time_samples()?
                .and_then(|samples| samples.into_iter().next().map(|(_, sampled)| sampled))
                .or(value))
        };
        time.map_or_else(default, |time| {
            attribute.get_at::<Value>(TimeCode::new(time))
        })
    };
    let vec3f = |name| match value(name)? {
        Some(Value::Vec3fVec(values)) => {
            Ok(Some(values.into_iter().map(|v| [v.x, v.y, v.z]).collect()))
        }
        Some(Value::Vec3dVec(values)) => Ok(Some(
            values
                .into_iter()
                .map(|v| [v.x as f32, v.y as f32, v.z as f32])
                .collect(),
        )),
        None => Ok(None),
        _ => anyhow::bail!("invalid {name} array"),
    };
    let Some(positions) = vec3f("positions")? else {
        return Ok(None);
    };
    let proto_indices = match value("protoIndices")? {
        Some(Value::IntVec(values)) => values,
        None => return Ok(None),
        _ => anyhow::bail!("invalid protoIndices array"),
    };
    let prototypes = owner.relationship("prototypes").targets()?;
    let orientations = match value("orientations")? {
        Some(Value::QuathVec(values)) => values
            .into_iter()
            .map(|q| [q.w.to_f32(), q.x.to_f32(), q.y.to_f32(), q.z.to_f32()])
            .collect(),
        Some(Value::QuatfVec(values)) => values.into_iter().map(|q| [q.w, q.x, q.y, q.z]).collect(),
        Some(Value::QuatdVec(values)) => values
            .into_iter()
            .map(|q| [q.w as f32, q.x as f32, q.y as f32, q.z as f32])
            .collect(),
        None => Vec::new(),
        _ => anyhow::bail!("invalid orientations array"),
    };
    let scales = vec3f("scales")?.unwrap_or_default();
    let ids = match value("ids")? {
        Some(Value::Int64Vec(values)) => Some(values),
        Some(Value::IntVec(values)) => Some(values.into_iter().map(i64::from).collect()),
        None => None,
        _ => anyhow::bail!("invalid ids array"),
    };
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
