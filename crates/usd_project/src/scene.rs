use serde::{Deserialize, Serialize};

use crate::{ModelId, SceneId, SceneMemberId};

/// The stage up-axis used by a USD source or canonical USDHub layer.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum StageUpAxis {
    Y,
    Z,
}

/// Source stage metrics captured during the read-only import inspection.
///
/// The authored flags deliberately remain part of the DTO so the UI can
/// distinguish an explicit source convention from OpenUSD's fallbacks.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct SourceSpatialConvention {
    pub up_axis: StageUpAxis,
    pub meters_per_unit: f64,
    pub up_axis_was_authored: bool,
    pub meters_per_unit_was_authored: bool,
}

impl Default for SourceSpatialConvention {
    fn default() -> Self {
        Self {
            up_axis: StageUpAxis::Y,
            meters_per_unit: 0.01,
            up_axis_was_authored: false,
            meters_per_unit_was_authored: false,
        }
    }
}

/// The stable object identity targeted by one Scene placement.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum SceneMemberTarget {
    Scene(SceneId),
    Model(ModelId),
}

/// One placement relationship owned by a Scene.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SceneMember {
    pub id: SceneMemberId,
    pub target: SceneMemberTarget,
    pub name: Option<String>,
    #[serde(default)]
    pub transform: ScenePlacementTransform,
}

/// One canonical row-major USD `matrix4d` placement transform.
///
/// USD uses row vectors, so translation is stored in the final row at indices
/// 12, 13, and 14. Callers should use the constructors below instead of
/// assembling the flat array by hand.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct ScenePlacementTransform(pub [f64; 16]);

impl ScenePlacementTransform {
    pub const IDENTITY: Self = Self([
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0, //
    ]);

    pub const fn identity() -> Self {
        Self::IDENTITY
    }

    pub fn from_translation(translation: [f64; 3]) -> Self {
        let mut transform = Self::IDENTITY;
        transform.0[12] = translation[0];
        transform.0[13] = translation[1];
        transform.0[14] = translation[2];
        transform
    }

    /// Compose `scale`, `rotation`, then `translation` for USD row vectors.
    /// The quaternion is supplied in USD's `(w, x, y, z)` order.
    pub fn from_trs(translation: [f64; 3], rotation_wxyz: [f64; 4], scale: [f64; 3]) -> Self {
        let [w, x, y, z] = normalized_quaternion(rotation_wxyz);
        let xx = x * x;
        let yy = y * y;
        let zz = z * z;
        let xy = x * y;
        let xz = x * z;
        let yz = y * z;
        let wx = w * x;
        let wy = w * y;
        let wz = w * z;
        let [sx, sy, sz] = scale;

        Self([
            sx * (1.0 - 2.0 * (yy + zz)),
            sx * 2.0 * (xy + wz),
            sx * 2.0 * (xz - wy),
            0.0,
            sy * 2.0 * (xy - wz),
            sy * (1.0 - 2.0 * (xx + zz)),
            sy * 2.0 * (yz + wx),
            0.0,
            sz * 2.0 * (xz + wy),
            sz * 2.0 * (yz - wx),
            sz * (1.0 - 2.0 * (xx + yy)),
            0.0,
            translation[0],
            translation[1],
            translation[2],
            1.0,
        ])
    }
}

fn normalized_quaternion([w, x, y, z]: [f64; 4]) -> [f64; 4] {
    let length = (w * w + x * x + y * y + z * z).sqrt();
    if length > f64::EPSILON {
        [w / length, x / length, y / length, z / length]
    } else {
        [1.0, 0.0, 0.0, 0.0]
    }
}

impl Default for ScenePlacementTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}
