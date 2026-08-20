use openusd::sdf::{Path, Value};
use openusd::usd::Stage;

use super::util::read_token;

// ── Imageable / model metadata ──────────────────────────────────────────

pub fn read_purpose(stage: &Stage, prim: &Path) -> anyhow::Result<String> {
    Ok(read_token(stage, prim, "purpose")?.unwrap_or_else(|| "default".to_string()))
}

/// Resolve the effective (inherited) `purpose` for `prim`: the closest ancestor
/// with an authored `purpose`, falling back to `"default"`. Mirrors
/// `UsdGeomImageable::ComputePurpose` — `purpose` is a *pruning, inherited*
/// token, so a `Scope` authored as `proxy` makes its whole subtree proxy.
pub fn read_effective_purpose(stage: &Stage, prim: &Path) -> anyhow::Result<String> {
    let mut cur = prim.clone();
    loop {
        if let Some(t) = read_token(stage, &cur, "purpose")? {
            return Ok(t);
        }
        match cur.parent() {
            Some(p) => cur = p,
            None => return Ok("default".to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibilityState {
    Inherited,
    Invisible,
}

pub fn read_visibility(stage: &Stage, prim: &Path) -> anyhow::Result<VisibilityState> {
    Ok(match read_token(stage, prim, "visibility")?.as_deref() {
        Some("invisible") => VisibilityState::Invisible,
        _ => VisibilityState::Inherited,
    })
}

pub fn read_kind(stage: &Stage, prim: &Path) -> anyhow::Result<Option<String>> {
    Ok(stage
        .prim(prim.clone())
        .kind()?
        .map(|t| t.as_str().to_string()))
}

// ── Custom data ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum CustomAttrValue {
    Bool(bool),
    Uchar(u8),
    Int(i32),
    UInt(u32),
    Int64(i64),
    UInt64(u64),
    Float(f32),
    Double(f64),
    String(String),
    Token(String),
    AssetPath(String),
    Vec2f([f32; 2]),
    Vec2d([f64; 2]),
    Vec2i([i32; 2]),
    Vec3f([f32; 3]),
    Vec3d([f64; 3]),
    Vec3i([i32; 3]),
    Vec4f([f32; 4]),
    Vec4d([f64; 4]),
    Vec4i([i32; 4]),
    Quatf([f32; 4]),
    Quatd([f64; 4]),
    Matrix4d([f64; 16]),
    BoolArray(Vec<bool>),
    UcharArray(Vec<u8>),
    IntArray(Vec<i32>),
    UIntArray(Vec<u32>),
    Int64Array(Vec<i64>),
    UInt64Array(Vec<u64>),
    FloatArray(Vec<f32>),
    DoubleArray(Vec<f64>),
    StringArray(Vec<String>),
    TokenArray(Vec<String>),
    PathArray(Vec<String>),
    Vec2fArray(Vec<[f32; 2]>),
    Vec3fArray(Vec<[f32; 3]>),
    Vec4fArray(Vec<[f32; 4]>),
    Vec2dArray(Vec<[f64; 2]>),
    Vec3dArray(Vec<[f64; 3]>),
    Vec4dArray(Vec<[f64; 4]>),
    QuatfArray(Vec<[f32; 4]>),
    Matrix4dArray(Vec<[f64; 16]>),
    Dict(CustomDict),
    TimeSamples(Vec<(f64, Box<CustomAttrValue>)>),
    Other(String),
}

impl CustomAttrValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<i64> {
        Some(match self {
            Self::Uchar(u) => *u as i64,
            Self::Int(i) => *i as i64,
            Self::UInt(u) => *u as i64,
            Self::Int64(i) => *i,
            Self::UInt64(u) => *u as i64,
            _ => return None,
        })
    }
    pub fn as_float(&self) -> Option<f64> {
        Some(match self {
            Self::Float(f) => *f as f64,
            Self::Double(d) => *d,
            Self::Int(i) => *i as f64,
            Self::Int64(i) => *i as f64,
            Self::UInt(u) => *u as f64,
            Self::UInt64(u) => *u as f64,
            Self::Uchar(u) => *u as f64,
            _ => return None,
        })
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) | Self::Token(s) | Self::AssetPath(s) => Some(s.as_str()),
            _ => None,
        }
    }
    pub fn as_vec2(&self) -> Option<[f32; 2]> {
        match self {
            Self::Vec2f(a) => Some(*a),
            Self::Vec2d(a) => Some([a[0] as f32, a[1] as f32]),
            Self::Vec2i(a) => Some([a[0] as f32, a[1] as f32]),
            _ => None,
        }
    }
    pub fn as_vec3(&self) -> Option<[f32; 3]> {
        match self {
            Self::Vec3f(a) => Some(*a),
            Self::Vec3d(a) => Some([a[0] as f32, a[1] as f32, a[2] as f32]),
            Self::Vec3i(a) => Some([a[0] as f32, a[1] as f32, a[2] as f32]),
            _ => None,
        }
    }
    pub fn as_vec4(&self) -> Option<[f32; 4]> {
        match self {
            Self::Vec4f(a) | Self::Quatf(a) => Some(*a),
            Self::Vec4d(a) | Self::Quatd(a) => {
                Some([a[0] as f32, a[1] as f32, a[2] as f32, a[3] as f32])
            }
            Self::Vec4i(a) => Some([a[0] as f32, a[1] as f32, a[2] as f32, a[3] as f32]),
            _ => None,
        }
    }
    pub fn as_matrix4(&self) -> Option<[f32; 16]> {
        match self {
            Self::Matrix4d(m) => {
                let mut out = [0.0f32; 16];
                for (i, slot) in out.iter_mut().enumerate() {
                    *slot = m[i] as f32;
                }
                Some(out)
            }
            _ => None,
        }
    }
    pub fn as_dict(&self) -> Option<&CustomDict> {
        match self {
            Self::Dict(d) => Some(d),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CustomDict {
    pub entries: Vec<(String, CustomAttrValue)>,
}

impl CustomDict {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn get(&self, name: &str) -> Option<&CustomAttrValue> {
        self.entries.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
    pub fn iter(&self) -> impl Iterator<Item = &(String, CustomAttrValue)> {
        self.entries.iter()
    }
    pub fn get_nested(&self, dotted: &str) -> Option<&CustomAttrValue> {
        let mut cur: &CustomAttrValue = self.get(dotted.split('.').next()?)?;
        for part in dotted.split('.').skip(1) {
            match cur {
                CustomAttrValue::Dict(d) => cur = d.get(part)?,
                _ => return None,
            }
        }
        Some(cur)
    }
}

/// Read every authored `custom` attribute on `prim`.
pub fn read_custom_attrs(
    stage: &Stage,
    prim: &Path,
) -> anyhow::Result<Vec<(String, CustomAttrValue)>> {
    let mut out = Vec::new();
    for name in stage.prim(prim.clone()).property_names()? {
        let attr = stage.prim(prim.clone()).attribute(&name);
        let is_custom = matches!(
            attr.get_metadata::<bool>("custom").ok().flatten(),
            Some(true)
        );
        if !is_custom {
            continue;
        }
        let Some(raw) = attr.get::<Value>()? else {
            continue;
        };
        out.push((name.to_string(), value_to_custom(raw)));
    }
    Ok(out)
}

fn value_to_custom(v: Value) -> CustomAttrValue {
    use CustomAttrValue as C;
    match v {
        Value::Bool(b) => C::Bool(b),
        Value::Uchar(u) => C::Uchar(u),
        Value::Int(i) => C::Int(i),
        Value::Uint(u) => C::UInt(u),
        Value::Int64(i) => C::Int64(i),
        Value::Uint64(u) => C::UInt64(u),
        Value::Half(h) => C::Float(f32::from(h)),
        Value::Float(f) => C::Float(f),
        Value::Double(d) => C::Double(d),
        Value::String(s) => C::String(s),
        Value::Token(s) => C::Token(s.as_str().to_string()),
        Value::AssetPath(s) => C::AssetPath(s.as_str().to_string()),
        Value::Vec2h(a) => C::Vec2f([f32::from(a.x), f32::from(a.y)]),
        Value::Vec2f(a) => C::Vec2f([a.x, a.y]),
        Value::Vec2d(a) => C::Vec2d([a.x, a.y]),
        Value::Vec2i(a) => C::Vec2i([a.x, a.y]),
        Value::Vec3h(a) => C::Vec3f([f32::from(a.x), f32::from(a.y), f32::from(a.z)]),
        Value::Vec3f(a) => C::Vec3f([a.x, a.y, a.z]),
        Value::Vec3d(a) => C::Vec3d([a.x, a.y, a.z]),
        Value::Vec3i(a) => C::Vec3i([a.x, a.y, a.z]),
        Value::Vec4h(a) => C::Vec4f([
            f32::from(a.x),
            f32::from(a.y),
            f32::from(a.z),
            f32::from(a.w),
        ]),
        Value::Vec4f(a) => C::Vec4f([a.x, a.y, a.z, a.w]),
        Value::Vec4d(a) => C::Vec4d([a.x, a.y, a.z, a.w]),
        Value::Vec4i(a) => C::Vec4i([a.x, a.y, a.z, a.w]),
        Value::Quath(q) => C::Quatf([
            f32::from(q.w),
            f32::from(q.x),
            f32::from(q.y),
            f32::from(q.z),
        ]),
        Value::Quatf(q) => C::Quatf([q.w, q.x, q.y, q.z]),
        Value::Quatd(q) => C::Quatd([q.w, q.x, q.y, q.z]),
        Value::Matrix4d(m) => C::Matrix4d(m.0),
        Value::BoolVec(v) => C::BoolArray(v),
        Value::UcharVec(v) => C::UcharArray(v),
        Value::IntVec(v) => C::IntArray(v),
        Value::UintVec(v) => C::UIntArray(v),
        Value::Int64Vec(v) => C::Int64Array(v),
        Value::Uint64Vec(v) => C::UInt64Array(v),
        Value::HalfVec(v) => C::FloatArray(v.into_iter().map(f32::from).collect()),
        Value::FloatVec(v) => C::FloatArray(v),
        Value::DoubleVec(v) => C::DoubleArray(v),
        Value::StringVec(v) => C::StringArray(v),
        Value::TokenVec(v) => {
            C::TokenArray(v.into_iter().map(|t| t.as_str().to_string()).collect())
        }
        Value::PathVec(v) => C::PathArray(v.into_iter().map(|p| p.as_str().to_string()).collect()),
        Value::Vec2fVec(v) => C::Vec2fArray(v.into_iter().map(|a| [a.x, a.y]).collect()),
        Value::Vec2dVec(v) => C::Vec2dArray(v.into_iter().map(|a| [a.x, a.y]).collect()),
        Value::Vec3fVec(v) => C::Vec3fArray(v.into_iter().map(|a| [a.x, a.y, a.z]).collect()),
        Value::Vec3dVec(v) => C::Vec3dArray(v.into_iter().map(|a| [a.x, a.y, a.z]).collect()),
        Value::Vec4fVec(v) => C::Vec4fArray(v.into_iter().map(|a| [a.x, a.y, a.z, a.w]).collect()),
        Value::Vec4dVec(v) => C::Vec4dArray(v.into_iter().map(|a| [a.x, a.y, a.z, a.w]).collect()),
        Value::QuatfVec(v) => C::QuatfArray(v.into_iter().map(|q| [q.w, q.x, q.y, q.z]).collect()),
        Value::Matrix4dVec(v) => C::Matrix4dArray(v.into_iter().map(|m| m.0).collect()),
        Value::Dictionary(dict) => C::Dict(dict_from_value_map(dict)),
        Value::TimeSamples(samples) => C::TimeSamples(
            samples
                .into_iter()
                .map(|(t, v)| (t, Box::new(value_to_custom(v))))
                .collect(),
        ),
        other => C::Other(format!("{other:?}")),
    }
}

fn dict_from_value_map(map: std::collections::HashMap<String, Value>) -> CustomDict {
    let mut entries: Vec<(String, CustomAttrValue)> = map
        .into_iter()
        .map(|(k, v)| (k, value_to_custom(v)))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    CustomDict { entries }
}

fn dict_from_value(raw: Option<Value>) -> Option<CustomDict> {
    match raw {
        Some(Value::Dictionary(d)) => {
            let dict = dict_from_value_map(d);
            (!dict.is_empty()).then_some(dict)
        }
        _ => None,
    }
}

pub fn read_custom_data(stage: &Stage, prim: &Path) -> anyhow::Result<Option<CustomDict>> {
    Ok(dict_from_value(stage.prim(prim.clone()).custom_data()?))
}

/// `assetInfo` dictionary on a prim (package-management metadata). Read via
/// the fork's public `Stage::metadata` accessor.
pub fn read_asset_info(stage: &Stage, prim: &Path) -> anyhow::Result<Option<CustomDict>> {
    // `assetInfo` has no public per-prim metadata accessor upstream yet. TODO restore.
    let _ = (stage, prim);
    Ok(None)
}

pub fn read_custom_layer_data(stage: &Stage) -> anyhow::Result<Option<CustomDict>> {
    Ok(dict_from_value(stage.custom_layer_data()?))
}
