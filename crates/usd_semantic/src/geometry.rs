//! Renderer-independent geometry signatures.

use anyhow::Result;
use openusd::sdf::{Path, Value};
use openusd::usd::Stage;
use usd_model::{Bounds3, GeometrySignature, HashDigest, QuantizedPoint3};

use crate::config::SemanticConfig;

pub fn extract_geometry(
    stage: &Stage,
    path: &Path,
    config: &SemanticConfig,
) -> Result<Option<GeometrySignature>> {
    let type_name = stage
        .prim(path.clone())
        .type_name()?
        .map(|value| value.as_str().to_owned());
    if type_name.as_deref() != Some("Mesh") {
        return Ok(None);
    }

    let prim = stage.prim(path.clone());
    let Some(points) = read_points(&prim.attribute("points").get::<Value>()?) else {
        return Ok(None);
    };
    let indices =
        read_ints(&prim.attribute("faceVertexIndices").get::<Value>()?).unwrap_or_default();
    let counts = read_ints(&prim.attribute("faceVertexCounts").get::<Value>()?).unwrap_or_default();
    let (bounds, centroid) = bounds_and_centroid(&points);
    let topology_hash = hash_topology(&counts, &indices);
    let shape_hash = hash_shape(&points, config.geometry_quantization);

    Ok(Some(GeometrySignature {
        vertex_count: points.len().try_into().unwrap_or(u32::MAX),
        index_count: indices.len().try_into().unwrap_or(u32::MAX),
        local_bounds: bounds,
        local_centroid: QuantizedPoint3(
            centroid.map(|value| quantize(value * config.geometry_quantization)),
        ),
        topology_hash,
        shape_hash,
        render_blob: None,
    }))
}

fn read_points(value: &Option<Value>) -> Option<Vec<[f64; 3]>> {
    match value.as_ref()? {
        Value::Vec3fVec(values) => Some(
            values
                .iter()
                .map(|value| [f64::from(value.x), f64::from(value.y), f64::from(value.z)])
                .collect(),
        ),
        Value::Vec3dVec(values) => Some(
            values
                .iter()
                .map(|value| [value.x, value.y, value.z])
                .collect(),
        ),
        _ => None,
    }
}

fn read_ints(value: &Option<Value>) -> Option<Vec<i64>> {
    match value.as_ref()? {
        Value::IntVec(values) => Some(values.iter().map(|value| i64::from(*value)).collect()),
        Value::Int64Vec(values) => Some(values.clone()),
        _ => None,
    }
}

fn bounds_and_centroid(points: &[[f64; 3]]) -> (Bounds3, [f64; 3]) {
    if points.is_empty() {
        return (
            Bounds3 {
                min: [0.0; 3],
                max: [0.0; 3],
            },
            [0.0; 3],
        );
    }
    let mut min = points[0];
    let mut max = points[0];
    let mut sum = [0.0; 3];
    for point in points {
        for axis in 0..3 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
            sum[axis] += point[axis];
        }
    }
    let count = points.len() as f64;
    (Bounds3 { min, max }, sum.map(|value| value / count))
}

fn hash_topology(counts: &[i64], indices: &[i64]) -> HashDigest {
    let mut bytes = Vec::new();
    write_values(&mut bytes, counts);
    write_values(&mut bytes, indices);
    digest(&bytes)
}

fn hash_shape(points: &[[f64; 3]], quantum: f64) -> HashDigest {
    let mut bytes = Vec::with_capacity(points.len() * 24);
    for point in points {
        for value in point {
            bytes.extend_from_slice(&quantize(value * quantum).to_le_bytes());
        }
    }
    digest(&bytes)
}

fn write_values(bytes: &mut Vec<u8>, values: &[i64]) {
    bytes.extend_from_slice(&(values.len() as u64).to_le_bytes());
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

fn quantize(value: f64) -> i64 {
    if value.is_finite() {
        value.round() as i64
    } else if value.is_sign_negative() {
        i64::MIN
    } else {
        i64::MAX
    }
}

fn digest(bytes: &[u8]) -> HashDigest {
    HashDigest::new(*blake3::hash(bytes).as_bytes())
}
