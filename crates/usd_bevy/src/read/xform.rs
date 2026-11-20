//! Xform reader — compose `xformOpOrder` into a single 4×4 and decompose to
//! TRS, read from the composed stage via openusd (`XformQuery` + fallback).

use bevy::math::{Affine3A, Mat3A, Mat4, Quat, Vec3, Vec3A};
use openusd::sdf::Path;
use openusd::usd::{Stage, TimeCode};
use openusd::vt::Value;
use openusd_schemas::geom::XformQuery;

/// Decomposed local transform: translate, rotate (quaternion `xyzw`), scale.
#[derive(Debug, Clone, Copy)]
pub struct Transform3 {
    pub translate: [f32; 3],
    pub rotate: [f32; 4],
    pub scale: [f32; 3],
}

/// Complete local transform evaluation including full affine matrix,
/// `resetXformStack` (`!resetXformStack!`), residual non-TRS factor (`U40`),
/// and full local affine override when `trs` is near-singular or cannot
/// reconstruct the local affine losslessly (`F-C03-I1-02`, `U40`, `U41`).
#[derive(Debug, Clone, Copy)]
pub struct LocalXformEvaluation {
    pub trs: Transform3,
    pub matrix: Mat4,
    pub residual_affine: Option<Affine3A>,
    pub full_local_affine: Option<Affine3A>,
    pub resets_xform_stack: bool,
}

#[derive(Clone)]
pub(crate) struct TransformBinding {
    prim: Path,
    query: Option<XformQuery>,
    ops: Vec<BoundXformOp>,
    resets_xform_stack: bool,
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
    Ok(stage
        .prim(prim.clone())?
        .attribute(name)
        .get_at::<Value>(time)?)
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
    Ok(read_local_xform_at(stage, prim, time)?.map(|eval| eval.trs))
}

/// Evaluate the prim's local transform at `time`, returning TRS decomposition,
/// the full 4×4 matrix, any non-TRS residual affine (`U40`), and
/// `resets_xform_stack` (`U41`).
pub fn read_local_xform_at(
    stage: &Stage,
    prim: &Path,
    time: Option<f64>,
) -> anyhow::Result<Option<LocalXformEvaluation>> {
    let Some(binding) = bind_transform(stage, prim)? else {
        return Ok(None);
    };
    read_bound_local_xform_at(stage, &binding, time)
}

/// Bind the authored xform operation order and operation kinds once. The
/// returned representation contains only the structural data needed by
/// [`read_bound_transform_at`]; sampled values are still read at playback
/// time.
pub(crate) fn bind_transform(
    stage: &Stage,
    prim: &Path,
) -> anyhow::Result<Option<TransformBinding>> {
    let prim_ref = stage.prim(prim.clone())?;
    let Some(raw) = attr_value(stage, prim, "xformOpOrder", None)? else {
        return Ok(None);
    };
    let order: Vec<String> = match raw {
        Value::TokenVec(v) => v.into_iter().map(|t| t.as_str().to_owned()).collect(),
        Value::StringVec(v) => v,
        Value::TokenListOp(op) => op
            .flatten()
            .into_iter()
            .map(|t| t.as_str().to_owned())
            .collect(),
        _ => return Ok(None),
    };
    let query = openusd_schemas::geom::Xformable::from_prim(prim_ref)
        .ok()
        .flatten()
        .and_then(|xf| XformQuery::new(&xf).ok());
    let resets_xform_stack = query
        .as_ref()
        .map(XformQuery::resets_xform_stack)
        .unwrap_or_else(|| order.iter().any(|t| t == "!resetXformStack!"));
    let active_order: Vec<String> = match order.iter().rposition(|t| t == "!resetXformStack!") {
        Some(idx) => order[idx + 1..].to_vec(),
        None => order,
    };
    Ok(Some(TransformBinding {
        prim: prim.clone(),
        query,
        ops: active_order.into_iter().map(bind_op).collect(),
        resets_xform_stack,
    }))
}

#[allow(dead_code)]
/// Read only the sampled operation values for a previously bound transform.
pub(crate) fn read_bound_transform_at(
    stage: &Stage,
    binding: &TransformBinding,
    time: Option<f64>,
) -> anyhow::Result<Option<Transform3>> {
    Ok(read_bound_local_xform_at(stage, binding, time)?.map(|eval| eval.trs))
}

pub(crate) fn read_bound_local_xform_at(
    stage: &Stage,
    binding: &TransformBinding,
    time: Option<f64>,
) -> anyhow::Result<Option<LocalXformEvaluation>> {
    let tc = time.map(TimeCode::new);
    let m = if let Some(query) = &binding.query {
        let mat4d = query.local_transform(tc)?;
        validate_and_convert_mat4d(&mat4d.0, &binding.prim)?
    } else {
        let mut m = Mat4::IDENTITY;
        for op in &binding.ops {
            let op_m = build_bound_op_matrix(stage, &binding.prim, op, tc)?;
            m *= op_m;
        }
        validate_glam_affine(&m, &binding.prim)?;
        m
    };

    let (trs, residual_affine, full_local_affine) = decompose_local_affine(m, &binding.prim)?;
    Ok(Some(LocalXformEvaluation {
        trs,
        matrix: m,
        residual_affine,
        full_local_affine,
        resets_xform_stack: binding.resets_xform_stack,
    }))
}

fn validate_and_convert_mat4d(raw: &[f64; 16], prim: &Path) -> anyhow::Result<Mat4> {
    if !raw.iter().all(|v| v.is_finite()) {
        anyhow::bail!("non-finite USD local transform matrix on {prim}");
    }
    // In row-major row-vector (USD) / column-major column-vector (glam) memory
    // layout, indices 3, 7, 11 are the homogeneous projective terms and index 15
    // is the homogeneous w scale.
    if raw[3].abs() > 1e-6
        || raw[7].abs() > 1e-6
        || raw[11].abs() > 1e-6
        || (raw[15] - 1.0).abs() > 1e-6
    {
        anyhow::bail!("projective (non-affine) USD local transform matrix unsupported on {prim}");
    }
    let cols: [f32; 16] = std::array::from_fn(|i| raw[i] as f32);
    if !cols.iter().all(|v| v.is_finite()) {
        anyhow::bail!("USD local transform matrix overflows f32 render precision on {prim}");
    }
    Ok(Mat4::from_cols_array(&cols))
}

fn validate_glam_affine(m: &Mat4, prim: &Path) -> anyhow::Result<()> {
    let cols = m.to_cols_array();
    if !cols.iter().all(|v| v.is_finite()) {
        anyhow::bail!("non-finite USD local transform matrix on {prim}");
    }
    if cols[3].abs() > 1e-5
        || cols[7].abs() > 1e-5
        || cols[11].abs() > 1e-5
        || (cols[15] - 1.0).abs() > 1e-5
    {
        anyhow::bail!("projective (non-affine) USD local transform matrix unsupported on {prim}");
    }
    Ok(())
}

fn mat4_to_affine3a(m: Mat4) -> Affine3A {
    let cols = m.to_cols_array();
    Affine3A {
        matrix3: Mat3A::from_cols(
            Vec3A::new(cols[0], cols[1], cols[2]),
            Vec3A::new(cols[4], cols[5], cols[6]),
            Vec3A::new(cols[8], cols[9], cols[10]),
        ),
        translation: Vec3A::new(cols[12], cols[13], cols[14]),
    }
}

fn affine_matches(expected: Affine3A, actual: Affine3A) -> bool {
    // Translation is copied into TRS exactly. Allow f32 roundoff from TRS
    // decomposition and recomposition separately for each linear basis column.
    const ROUND_OFF: f32 = 8.0 * f32::EPSILON;
    let expected_linear = expected.matrix3;
    let actual_linear = actual.matrix3;
    expected.translation == actual.translation
        && [
            (expected_linear.x_axis, actual_linear.x_axis),
            (expected_linear.y_axis, actual_linear.y_axis),
            (expected_linear.z_axis, actual_linear.z_axis),
        ]
        .into_iter()
        .all(|(expected, actual)| {
            actual.is_finite()
                && (expected - actual).abs().max_element()
                    <= ROUND_OFF * expected.abs().max_element()
        })
}

fn safe_fallback_trs(full: Mat4) -> Transform3 {
    let cols = full.to_cols_array();
    let t = [cols[12], cols[13], cols[14]];
    let sx = Vec3::new(cols[0], cols[1], cols[2]).length();
    let sy = Vec3::new(cols[4], cols[5], cols[6]).length();
    let sz = Vec3::new(cols[8], cols[9], cols[10]).length();
    if full.determinant().abs() > 1e-6 {
        let (s, r, tr) = full.to_scale_rotation_translation();
        if s.is_finite() && r.is_finite() && tr.is_finite() {
            return Transform3 {
                translate: [tr.x, tr.y, tr.z],
                rotate: [r.x, r.y, r.z, r.w],
                scale: [s.x, s.y, s.z],
            };
        }
    }
    Transform3 {
        translate: t,
        rotate: [0.0, 0.0, 0.0, 1.0],
        scale: [sx, sy, sz],
    }
}

fn decompose_local_affine(
    full: Mat4,
    prim: &Path,
) -> anyhow::Result<(Transform3, Option<Affine3A>, Option<Affine3A>)> {
    validate_glam_affine(&full, prim)?;
    let full_affine = mat4_to_affine3a(full);
    let full = Mat4::from(full_affine);

    // `Mat4::to_scale_rotation_translation` divides by scale components and
    // panics or produces NaNs when determinant is zero/near-zero. Guard with
    // determinant magnitude first.
    let full_det = full.determinant();
    if full_det.abs() > 1e-6 {
        let (s, r, t) = full.to_scale_rotation_translation();
        if s.is_finite() && r.is_finite() && t.is_finite() {
            let trs = Transform3 {
                translate: [t.x, t.y, t.z],
                rotate: [r.x, r.y, r.z, r.w],
                scale: [s.x, s.y, s.z],
            };
            let trs_affine = Affine3A::from_scale_rotation_translation(s, r, t);
            if affine_matches(full_affine, trs_affine) {
                return Ok((trs, None, None));
            }
            if trs_affine.matrix3.determinant().abs() > 1e-6 {
                // Both transforms already share the authored translation.
                // Solve only for the linear residual to avoid cancellation
                // from inverting and recomposing large world coordinates.
                let residual = Affine3A {
                    matrix3: trs_affine.matrix3.inverse() * full_affine.matrix3,
                    translation: Vec3A::ZERO,
                };
                if residual.matrix3.is_finite()
                    && affine_matches(full_affine, trs_affine * residual)
                {
                    return Ok((trs, Some(residual), None));
                }
            }
            return Ok((trs, None, Some(full_affine)));
        }
    }

    // Singular or near-singular matrix (e.g. zero or tiny scale on one axis,
    // potentially combined with shear): check whether pure axis-aligned TRS
    // without rotation/shear already equals `full`.
    let fallback_trs = safe_fallback_trs(full);
    let fallback_affine = Affine3A::from_scale_rotation_translation(
        Vec3::from_array(fallback_trs.scale),
        Quat::from_array(fallback_trs.rotate),
        Vec3::from_array(fallback_trs.translate),
    );
    if affine_matches(full_affine, fallback_affine) {
        return Ok((fallback_trs, None, None));
    }
    Ok((fallback_trs, None, Some(full_affine)))
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
        XformOpKind::Transform => match &raw {
            Value::Matrix4d(m) => validate_and_convert_mat4d(&m.0, prim)?,
            _ => Mat4::IDENTITY,
        },
        XformOpKind::Unknown => Mat4::IDENTITY,
    };

    if op.inverted {
        if m.determinant().abs() <= 1e-8 {
            anyhow::bail!("singular inverted xformOp {} on {prim}", op.name);
        }
        let inv = m.inverse();
        validate_glam_affine(&inv, prim)?;
        Ok(inv)
    } else {
        validate_glam_affine(&m, prim)?;
        Ok(m)
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
