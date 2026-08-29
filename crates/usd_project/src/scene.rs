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
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct ScenePlacementTransform(pub [f64; 16]);

impl ScenePlacementTransform {
    pub const IDENTITY: Self = Self([
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0, //
    ]);
}

impl Default for ScenePlacementTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}
