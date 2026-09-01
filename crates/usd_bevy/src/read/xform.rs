//! Xform reader — compose `xformOpOrder` into a single 4×4 and decompose to
//! TRS, read from the composed stage via openusd.

use glam::{Mat4, Quat, Vec3};
use openusd::sdf::{Path, Value};
use openusd::usd::{Stage, TimeCode};

/// Decomposed local transform: translate, rotate (quaternion `xyzw`), scale.
#[derive(Debug, Clone, Copy)]
pub struct Transform3 {
    pub translate: [f32; 3],
    pub rotate: [f32; 4],
    pub scale: [f32; 3],
}

#[derive(Clone)]
pub(crate) struct TransformBinding {
    prim: Path,
    ops: Vec<BoundXformOp>,
}

#[derive(Clone)]
struct BoundXformOp {
    name: String,
    kind: XformOpKind,
    inverted: bool,
}

#[derive(Clone, Copy)]
enum XformOpKind {
    Translate,
    Scale,
    Orient,
    RotateX,
    RotateY,
    RotateZ,
    RotateXYZ,
    RotateYXZ,
    RotateZXY,
    RotateXZY,
    RotateYZX,
    RotateZYX,
    Transform,
    Unknown,
}

fn attr_value(
    stage: &Stage,
    prim: &Path,
    name: &str,
    time: Option<TimeCode>,
) -> anyhow::Result<Option<Value>> {
    stage
        .prim(prim.clone())
        .attribute(name)
        .get_at::<Value>(time)
}

/// Read `xformOpOrder` and compose every listed op into a single 4×4, then
/// decompose to TRS, at the stage's default time. `None` when no
/// `xformOpOrder` is authored.
pub fn read_transform(stage: &Stage, prim: &Path) -> anyhow::Result<Option<Transform3>> {
    read_transform_at(stage, prim, None)
}

/// Like [`read_transform`], but resolves attribute values at `time` (a USD
/// time code). `None` reads the default (unanimated) value.
pub fn read_transform_at(
    stage: &Stage,
    prim: &Path,
    time: Option<f64>,
) -> anyhow::Result<Option<Transform3>> {
    let Some(binding) = bind_transform(stage, prim)? else {
        return Ok(None);
    };
    read_bound_transform_at(stage, &binding, time)
}

/// Bind the authored xform operation order and operation kinds once. The
/// returned representation contains only the structural data needed by
/// [`read_bound_transform_at`]; sampled values are still read at playback
/// time.
pub(crate) fn bind_transform(
    stage: &Stage,
    prim: &Path,
) -> anyhow::Result<Option<TransformBinding>> {
    let Some(raw) = attr_value(stage, prim, "xformOpOrder", None)? else {
        return Ok(None);
    };
    let order = match raw {
        Value::TokenVec(v) => v.into_iter().map(|t| t.as_str().to_owned()).collect(),
        Value::StringVec(v) => v,
        Value::TokenListOp(op) => op
            .flatten()
            .into_iter()
            .map(|t| t.as_str().to_owned())
            .collect(),
        _ => return Ok(None),
    };
    Ok(Some(TransformBinding {
        prim: prim.clone(),
        ops: order.into_iter().map(bind_op).collect(),
    }))
}

/// Read only the sampled operation values for a previously bound transform.
pub(crate) fn read_bound_transform_at(
    stage: &Stage,
    binding: &TransformBinding,
    time: Option<f64>,
) -> anyhow::Result<Option<Transform3>> {
    let tc = time.map(TimeCode::new);
    let mut m = Mat4::IDENTITY;
    for op in &binding.ops {
        m *= build_bound_op_matrix(stage, &binding.prim, op, tc)?;
    }

    let (s, r, t) = m.to_scale_rotation_translation();
    Ok(Some(Transform3 {
        translate: [t.x, t.y, t.z],
        rotate: [r.x, r.y, r.z, r.w],
        scale: [s.x, s.y, s.z],
    }))
}

fn bind_op(op_token: String) -> BoundXformOp {
    const INVERT: &str = "!invert!";
    let (inverted, base) = match op_token.strip_prefix(INVERT) {
        Some(stripped) => (true, stripped),
        None => (false, op_token.as_str()),
    };
    let kind = base.strip_prefix("xformOp:").unwrap_or(base);
    let kind = kind.split(':').next().unwrap_or(kind);
    let kind = match kind {
        "translate" => XformOpKind::Translate,
        "scale" => XformOpKind::Scale,
        "orient" => XformOpKind::Orient,
        "rotateX" => XformOpKind::RotateX,
        "rotateY" => XformOpKind::RotateY,
        "rotateZ" => XformOpKind::RotateZ,
        "rotateXYZ" => XformOpKind::RotateXYZ,
        "rotateYXZ" => XformOpKind::RotateYXZ,
        "rotateZXY" => XformOpKind::RotateZXY,
        "rotateXZY" => XformOpKind::RotateXZY,
        "rotateYZX" => XformOpKind::RotateYZX,
        "rotateZYX" => XformOpKind::RotateZYX,
        "transform" => XformOpKind::Transform,
        _ => XformOpKind::Unknown,
    };
    BoundXformOp {
        name: base.to_owned(),
        kind,
        inverted,
    }
}

fn build_bound_op_matrix(
    stage: &Stage,
    prim: &Path,
    op: &BoundXformOp,
    time: Option<TimeCode>,
) -> anyhow::Result<Mat4> {
    let Some(raw) = attr_value(stage, prim, &op.name, time)? else {
        return Ok(Mat4::IDENTITY);
    };

    let m = match op.kind {
        XformOpKind::Translate => {
            Mat4::from_translation(Vec3::from(value_to_vec3f(&raw).unwrap_or([0.0, 0.0, 0.0])))
        }
        XformOpKind::Scale => {
            Mat4::from_scale(Vec3::from(value_to_vec3f(&raw).unwrap_or([1.0, 1.0, 1.0])))
        }
        XformOpKind::Orient => {
            let q = value_to_quat_wxyz(&raw).unwrap_or([1.0, 0.0, 0.0, 0.0]);
            Mat4::from_quat(Quat::from_xyzw(q[1], q[2], q[3], q[0]))
        }
        XformOpKind::RotateX => {
            Mat4::from_rotation_x(value_to_scalar_f32(&raw).unwrap_or(0.0).to_radians())
        }
        XformOpKind::RotateY => {
            Mat4::from_rotation_y(value_to_scalar_f32(&raw).unwrap_or(0.0).to_radians())
        }
        XformOpKind::RotateZ => {
            Mat4::from_rotation_z(value_to_scalar_f32(&raw).unwrap_or(0.0).to_radians())
        }
        XformOpKind::RotateXYZ
        | XformOpKind::RotateYXZ
        | XformOpKind::RotateZXY
        | XformOpKind::RotateXZY
        | XformOpKind::RotateYZX
        | XformOpKind::RotateZYX => {
            let v = value_to_vec3f(&raw).unwrap_or([0.0, 0.0, 0.0]);
            let rx_m = Mat4::from_rotation_x(v[0].to_radians());
            let ry_m = Mat4::from_rotation_y(v[1].to_radians());
            let rz_m = Mat4::from_rotation_z(v[2].to_radians());
            match op.kind {
                XformOpKind::RotateXYZ => rz_m * ry_m * rx_m,
                XformOpKind::RotateYXZ => rz_m * rx_m * ry_m,
                XformOpKind::RotateZXY => ry_m * rx_m * rz_m,
                XformOpKind::RotateXZY => ry_m * rz_m * rx_m,
                XformOpKind::RotateYZX => rx_m * rz_m * ry_m,
                XformOpKind::RotateZYX => rx_m * ry_m * rz_m,
                _ => unreachable!(),
            }
        }
        XformOpKind::Transform => value_to_mat4_glam(&raw).unwrap_or(Mat4::IDENTITY),
        XformOpKind::Unknown => Mat4::IDENTITY,
    };

    Ok(if op.inverted { m.inverse() } else { m })
}

fn value_to_mat4_glam(v: &Value) -> Option<Mat4> {
    match v {
        Value::Matrix4d(m) => {
            let cols: [f32; 16] = std::array::from_fn(|i| m.0[i] as f32);
            Some(Mat4::from_cols_array(&cols))
        }
        _ => None,
    }
}

fn value_to_vec3f(v: &Value) -> Option<[f32; 3]> {
    match v {
        Value::Vec3f(a) => Some([a.x, a.y, a.z]),
        Value::Vec3d(a) => Some([a.x as f32, a.y as f32, a.z as f32]),
        _ => None,
    }
}

fn value_to_scalar_f32(v: &Value) -> Option<f32> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Double(d) => Some(*d as f32),
        Value::Int(i) => Some(*i as f32),
        Value::Int64(i) => Some(*i as f32),
        _ => None,
    }
}

fn value_to_quat_wxyz(v: &Value) -> Option<[f32; 4]> {
    match v {
        Value::Quatf(q) => Some([q.w, q.x, q.y, q.z]),
        Value::Quatd(q) => Some([q.w as f32, q.x as f32, q.y as f32, q.z as f32]),
        _ => None,
    }
}
