use viewport_protocol::EditorValue;

/// Converts a JSON-typed editor value into its USD SDF equivalent.
pub(super) fn editor_value_to_usd(
    type_name: &str,
    value: &EditorValue,
) -> Result<openusd::sdf::Value, String> {
    use openusd::sdf::Value;

    fn number(value: &EditorValue) -> Result<f64, String> {
        value
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(|| "editor value must be a finite JSON number".to_owned())
    }
    fn integer(value: &EditorValue) -> Result<i64, String> {
        value
            .as_i64()
            .ok_or_else(|| "editor value must be a JSON integer".to_owned())
    }
    fn boolean(value: &EditorValue) -> Result<bool, String> {
        value
            .as_bool()
            .ok_or_else(|| "editor value must be a JSON boolean".to_owned())
    }
    fn text(value: &EditorValue) -> Result<String, String> {
        value
            .as_str()
            .map(ToOwned::to_owned)
            .ok_or_else(|| "editor value must be a JSON string".to_owned())
    }
    fn numbers<const N: usize>(value: &EditorValue) -> Result<[f64; N], String> {
        let values = value
            .as_array()
            .ok_or_else(|| format!("editor value must be an array of {N} numbers"))?;
        if values.len() != N {
            return Err(format!("editor value must contain exactly {N} numbers"));
        }
        values
            .iter()
            .map(number)
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| format!("editor value must contain exactly {N} numbers"))
    }
    fn values(value: &EditorValue) -> Result<&[EditorValue], String> {
        value
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| "editor array value must be a JSON array".to_owned())
    }

    let value = match type_name {
        "bool" => Value::Bool(boolean(value)?),
        "uchar" => Value::Uchar(
            integer(value)?
                .try_into()
                .map_err(|_| "uchar is outside the range 0..255".to_owned())?,
        ),
        "int" => Value::Int(
            integer(value)?
                .try_into()
                .map_err(|_| "int is outside the i32 range".to_owned())?,
        ),
        "uint" => Value::Uint(
            integer(value)?
                .try_into()
                .map_err(|_| "uint is outside the u32 range".to_owned())?,
        ),
        "int64" => Value::Int64(integer(value)?),
        "uint64" => Value::Uint64(
            integer(value)?
                .try_into()
                .map_err(|_| "uint64 cannot be negative".to_owned())?,
        ),
        "float" => Value::Float(number(value)? as f32),
        "double" => Value::Double(number(value)?),
        "string" => Value::String(text(value)?),
        "token" => Value::Token(text(value)?.as_str().into()),
        "asset" => Value::AssetPath(openusd::sdf::AssetPath::new(text(value)?)),
        "timecode" => Value::TimeCode(openusd::sdf::TimeCode(number(value)?)),
        "float2" => Value::Vec2f(numbers::<2>(value)?.map(|v| v as f32).into()),
        "float3" | "point3f" | "vector3f" | "normal3f" | "color3f" => {
            Value::Vec3f(numbers::<3>(value)?.map(|v| v as f32).into())
        }
        "float4" | "color4f" => Value::Vec4f(numbers::<4>(value)?.map(|v| v as f32).into()),
        "double2" => Value::Vec2d(numbers::<2>(value)?.into()),
        "double3" | "point3d" | "vector3d" | "normal3d" | "color3d" => {
            Value::Vec3d(numbers::<3>(value)?.into())
        }
        "double4" | "color4d" => Value::Vec4d(numbers::<4>(value)?.into()),
        "int2" => Value::Vec2i(numbers::<2>(value)?.map(|v| v as i32).into()),
        "int3" => Value::Vec3i(numbers::<3>(value)?.map(|v| v as i32).into()),
        "int4" => Value::Vec4i(numbers::<4>(value)?.map(|v| v as i32).into()),
        "quatf" => Value::Quatf(numbers::<4>(value)?.map(|v| v as f32).into()),
        "quatd" => Value::Quatd(numbers::<4>(value)?.into()),
        "matrix2d" => Value::Matrix2d(numbers::<4>(value)?.into()),
        "matrix3d" => Value::Matrix3d(numbers::<9>(value)?.into()),
        "matrix4d" => Value::Matrix4d(numbers::<16>(value)?.into()),
        "path" => Value::PathExpression(text(value)?),
        "bool[]" => Value::BoolVec(
            values(value)?
                .iter()
                .map(boolean)
                .collect::<Result<_, _>>()?,
        ),
        "int[]" => Value::IntVec(
            values(value)?
                .iter()
                .map(|value| {
                    integer(value)?
                        .try_into()
                        .map_err(|_| "int[] contains an out-of-range value".to_owned())
                })
                .collect::<Result<_, _>>()?,
        ),
        "uint[]" => Value::UintVec(
            values(value)?
                .iter()
                .map(|value| {
                    integer(value)?
                        .try_into()
                        .map_err(|_| "uint[] contains an out-of-range value".to_owned())
                })
                .collect::<Result<_, _>>()?,
        ),
        "int64[]" => Value::Int64Vec(
            values(value)?
                .iter()
                .map(integer)
                .collect::<Result<_, _>>()?,
        ),
        "uint64[]" => Value::Uint64Vec(
            values(value)?
                .iter()
                .map(|value| {
                    integer(value)?
                        .try_into()
                        .map_err(|_| "uint64[] contains a negative value".to_owned())
                })
                .collect::<Result<_, _>>()?,
        ),
        "float[]" => Value::FloatVec(
            values(value)?
                .iter()
                .map(|value| Ok(number(value)? as f32))
                .collect::<Result<_, String>>()?,
        ),
        "double[]" => Value::DoubleVec(
            values(value)?
                .iter()
                .map(number)
                .collect::<Result<_, _>>()?,
        ),
        "string[]" => Value::StringVec(values(value)?.iter().map(text).collect::<Result<_, _>>()?),
        "token[]" => Value::TokenVec(
            values(value)?
                .iter()
                .map(|value| Ok(text(value)?.as_str().into()))
                .collect::<Result<_, String>>()?,
        ),
        "asset[]" => Value::AssetPathVec(
            values(value)?
                .iter()
                .map(|value| Ok(openusd::sdf::AssetPath::new(text(value)?)))
                .collect::<Result<_, String>>()?,
        ),
        "float3[]" => Value::Vec3fVec(
            values(value)?
                .iter()
                .map(|value| Ok(numbers::<3>(value)?.map(|v| v as f32).into()))
                .collect::<Result<_, String>>()?,
        ),
        "double3[]" => Value::Vec3dVec(
            values(value)?
                .iter()
                .map(|value| Ok(numbers::<3>(value)?.into()))
                .collect::<Result<_, String>>()?,
        ),
        "matrix4d[]" => Value::Matrix4dVec(
            values(value)?
                .iter()
                .map(|value| Ok(numbers::<16>(value)?.into()))
                .collect::<Result<_, String>>()?,
        ),
        _ => {
            return Err(format!(
                "unsupported USD editor attribute type: {type_name}"
            ));
        }
    };
    Ok(value)
}
