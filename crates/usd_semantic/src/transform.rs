//! USD xform-op composition and semantic quantization.

use anyhow::Result;
use openusd::sdf::{Path, Value};
use openusd::usd::Stage;
use usd_model::{HashDigest, TransformSignature};

use crate::config::SemanticConfig;

type Matrix4 = [f64; 16];

pub fn extract_transform(
    stage: &Stage,
    path: &Path,
    config: &SemanticConfig,
) -> Result<TransformSignature> {
    let matrix = compose_transform(stage, path)?;
    let (translation, rotation, scale) = decompose_matrix(matrix);
    let translation_mm =
        translation.map(|value| quantize_i64(value * config.translation_mm_per_unit));
    let rotation_quantized =
        rotation.map(|value| quantize_i32(value * config.rotation_quantization));
    let scale_quantized = scale.map(|value| quantize_i32(value * config.scale_quantization));

    let mut bytes = Vec::new();
    for value in translation_mm {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in rotation_quantized {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in scale_quantized {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    Ok(TransformSignature {
        translation_mm,
        rotation_quantized,
        scale_quantized,
        hash: digest(&bytes),
    })
}

fn compose_transform(stage: &Stage, path: &Path) -> Result<Matrix4> {
    let Some(raw) = stage
        .prim(path.clone())
        .attribute("xformOpOrder")
        .get::<Value>()?
    else {
        return Ok(identity());
    };

    let operations: Vec<String> = match raw {
        Value::TokenVec(values) => values
            .into_iter()
            .map(|value| value.as_str().to_owned())
            .collect(),
        Value::StringVec(values) => values,
        Value::TokenListOp(value) => value
            .flatten()
            .into_iter()
            .map(|value| value.as_str().to_owned())
            .collect(),
        _ => return Ok(identity()),
    };

    let mut matrix = identity();
    for operation in operations {
        matrix = multiply(matrix, operation_matrix(stage, path, &operation)?);
    }
    Ok(matrix)
}

fn operation_matrix(stage: &Stage, path: &Path, operation: &str) -> Result<Matrix4> {
    let (inverted, base) = operation
        .strip_prefix("!invert!")
        .map_or((false, operation), |value| (true, value));
    let value = stage.prim(path.clone()).attribute(base).get::<Value>()?;
    let Some(value) = value else {
        return Ok(identity());
    };
    let kind = base
        .strip_prefix("xformOp:")
        .unwrap_or(base)
        .split(':')
        .next()
        .unwrap_or(base);

    let matrix = match kind {
        "translate" => translation(value_to_vec3(&value).unwrap_or([0.0; 3])),
        "scale" => scale(value_to_vec3(&value).unwrap_or([1.0; 3])),
        "orient" => {
            rotation_from_quaternion(value_to_quaternion(&value).unwrap_or([1.0, 0.0, 0.0, 0.0]))
        }
        "rotateX" => rotation_x(value_to_scalar(&value).unwrap_or(0.0).to_radians()),
        "rotateY" => rotation_y(value_to_scalar(&value).unwrap_or(0.0).to_radians()),
        "rotateZ" => rotation_z(value_to_scalar(&value).unwrap_or(0.0).to_radians()),
        "rotateXYZ" | "rotateYXZ" | "rotateZXY" | "rotateXZY" | "rotateYZX" | "rotateZYX" => {
            let [x, y, z] = value_to_vec3(&value).unwrap_or([0.0; 3]);
            let rx = rotation_x(x.to_radians());
            let ry = rotation_y(y.to_radians());
            let rz = rotation_z(z.to_radians());
            match kind {
                "rotateXYZ" => multiply(multiply(rz, ry), rx),
                "rotateYXZ" => multiply(multiply(rz, rx), ry),
                "rotateZXY" => multiply(multiply(ry, rx), rz),
                "rotateXZY" => multiply(multiply(ry, rz), rx),
                "rotateYZX" => multiply(multiply(rx, rz), ry),
                "rotateZYX" => multiply(multiply(rx, ry), rz),
                _ => unreachable!(),
            }
        }
        "transform" => match value {
            Value::Matrix4d(value) => value.0,
            _ => identity(),
        },
        _ => identity(),
    };

    Ok(if inverted {
        inverse_affine(matrix)
    } else {
        matrix
    })
}

fn value_to_vec3(value: &Value) -> Option<[f64; 3]> {
    match value {
        Value::Vec3h(value) => Some([
            f64::from(f32::from(value.x)),
            f64::from(f32::from(value.y)),
            f64::from(f32::from(value.z)),
        ]),
        Value::Vec3f(value) => Some([f64::from(value.x), f64::from(value.y), f64::from(value.z)]),
        Value::Vec3d(value) => Some([value.x, value.y, value.z]),
        Value::Vec3i(value) => Some([f64::from(value.x), f64::from(value.y), f64::from(value.z)]),
        _ => None,
    }
}

fn value_to_scalar(value: &Value) -> Option<f64> {
    match value {
        Value::Half(value) => Some(f64::from(f32::from(*value))),
        Value::Float(value) => Some(f64::from(*value)),
        Value::Double(value) => Some(*value),
        Value::Int(value) => Some(f64::from(*value)),
        Value::Int64(value) => Some(*value as f64),
        _ => None,
    }
}

fn value_to_quaternion(value: &Value) -> Option<[f64; 4]> {
    match value {
        Value::Quath(value) => Some([
            f64::from(f32::from(value.w)),
            f64::from(f32::from(value.x)),
            f64::from(f32::from(value.y)),
            f64::from(f32::from(value.z)),
        ]),
        Value::Quatf(value) => Some([
            f64::from(value.w),
            f64::from(value.x),
            f64::from(value.y),
            f64::from(value.z),
        ]),
        Value::Quatd(value) => Some([value.w, value.x, value.y, value.z]),
        _ => None,
    }
}

fn identity() -> Matrix4 {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

fn translation([x, y, z]: [f64; 3]) -> Matrix4 {
    let mut matrix = identity();
    matrix[12] = x;
    matrix[13] = y;
    matrix[14] = z;
    matrix
}

fn scale([x, y, z]: [f64; 3]) -> Matrix4 {
    [
        x, 0.0, 0.0, 0.0, 0.0, y, 0.0, 0.0, 0.0, 0.0, z, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

fn rotation_x(angle: f64) -> Matrix4 {
    let (sin, cos) = angle.sin_cos();
    [
        1.0, 0.0, 0.0, 0.0, 0.0, cos, sin, 0.0, 0.0, -sin, cos, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

fn rotation_y(angle: f64) -> Matrix4 {
    let (sin, cos) = angle.sin_cos();
    [
        cos, 0.0, -sin, 0.0, 0.0, 1.0, 0.0, 0.0, sin, 0.0, cos, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

fn rotation_z(angle: f64) -> Matrix4 {
    let (sin, cos) = angle.sin_cos();
    [
        cos, sin, 0.0, 0.0, -sin, cos, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

fn rotation_from_quaternion([w, x, y, z]: [f64; 4]) -> Matrix4 {
    let length = (w * w + x * x + y * y + z * z).sqrt();
    if length <= f64::EPSILON {
        return identity();
    }
    let (w, x, y, z) = (w / length, x / length, y / length, z / length);
    [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + z * w),
        2.0 * (x * z - y * w),
        0.0,
        2.0 * (x * y - z * w),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z + x * w),
        0.0,
        2.0 * (x * z + y * w),
        2.0 * (y * z - x * w),
        1.0 - 2.0 * (x * x + y * y),
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

fn multiply(left: Matrix4, right: Matrix4) -> Matrix4 {
    std::array::from_fn(|index| {
        let row = index % 4;
        let column = index / 4;
        (0..4)
            .map(|k| left[k * 4 + row] * right[column * 4 + k])
            .sum()
    })
}

fn inverse_affine(matrix: Matrix4) -> Matrix4 {
    let (translation_value, rotation_value, scale_value) = decompose_matrix(matrix);
    let inverse_scale = [
        if scale_value[0].abs() > f64::EPSILON {
            1.0 / scale_value[0]
        } else {
            0.0
        },
        if scale_value[1].abs() > f64::EPSILON {
            1.0 / scale_value[1]
        } else {
            0.0
        },
        if scale_value[2].abs() > f64::EPSILON {
            1.0 / scale_value[2]
        } else {
            0.0
        },
    ];
    let inverse_rotation = rotation_from_quaternion([
        rotation_value[0],
        -rotation_value[1],
        -rotation_value[2],
        -rotation_value[3],
    ]);
    let inverse_translation = multiply(
        multiply(scale(inverse_scale), inverse_rotation),
        translation([
            -translation_value[0],
            -translation_value[1],
            -translation_value[2],
        ]),
    );
    inverse_translation
}

fn decompose_matrix(matrix: Matrix4) -> ([f64; 3], [f64; 4], [f64; 3]) {
    let translation = [matrix[12], matrix[13], matrix[14]];
    let mut scale = [
        length([matrix[0], matrix[1], matrix[2]]),
        length([matrix[4], matrix[5], matrix[6]]),
        length([matrix[8], matrix[9], matrix[10]]),
    ];
    if determinant3(matrix) < 0.0 {
        scale[0] = -scale[0];
    }
    let mut rotation = matrix;
    for (column, value) in scale.iter().enumerate() {
        if value.abs() > f64::EPSILON {
            for row in 0..3 {
                rotation[column * 4 + row] /= *value;
            }
        }
    }
    let trace = rotation[0] + rotation[5] + rotation[10];
    let mut quaternion = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [
            0.25 * s,
            (rotation[6] - rotation[9]) / s,
            (rotation[8] - rotation[2]) / s,
            (rotation[1] - rotation[4]) / s,
        ]
    } else if rotation[0] > rotation[5] && rotation[0] > rotation[10] {
        let s = (1.0 + rotation[0] - rotation[5] - rotation[10]).sqrt() * 2.0;
        [
            (rotation[6] - rotation[9]) / s,
            0.25 * s,
            (rotation[4] + rotation[1]) / s,
            (rotation[8] + rotation[2]) / s,
        ]
    } else if rotation[5] > rotation[10] {
        let s = (1.0 + rotation[5] - rotation[0] - rotation[10]).sqrt() * 2.0;
        [
            (rotation[8] - rotation[2]) / s,
            (rotation[4] + rotation[1]) / s,
            0.25 * s,
            (rotation[9] + rotation[6]) / s,
        ]
    } else {
        let s = (1.0 + rotation[10] - rotation[0] - rotation[5]).sqrt() * 2.0;
        [
            (rotation[1] - rotation[4]) / s,
            (rotation[8] + rotation[2]) / s,
            (rotation[9] + rotation[6]) / s,
            0.25 * s,
        ]
    };
    let length = length(quaternion);
    if length > f64::EPSILON {
        quaternion = quaternion.map(|value| value / length);
    }
    if quaternion[0] < 0.0 {
        quaternion = quaternion.map(|value| -value);
    }
    (translation, quaternion, scale)
}

fn determinant3(matrix: Matrix4) -> f64 {
    matrix[0] * (matrix[5] * matrix[10] - matrix[9] * matrix[6])
        - matrix[4] * (matrix[1] * matrix[10] - matrix[9] * matrix[2])
        + matrix[8] * (matrix[1] * matrix[6] - matrix[5] * matrix[2])
}

fn length<const N: usize>(values: [f64; N]) -> f64 {
    values.iter().map(|value| value * value).sum::<f64>().sqrt()
}

fn quantize_i64(value: f64) -> i64 {
    if value.is_finite() {
        value.round() as i64
    } else if value.is_sign_negative() {
        i64::MIN
    } else {
        i64::MAX
    }
}

fn quantize_i32(value: f64) -> i32 {
    quantize_i64(value).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn digest(bytes: &[u8]) -> HashDigest {
    HashDigest::new(*blake3::hash(bytes).as_bytes())
}
