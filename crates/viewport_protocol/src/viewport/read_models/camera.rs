use serde::{Deserialize, Serialize};

/// Renderer-neutral camera orientations published by the authoritative
/// viewport camera. The quaternion uses the conventional `[x, y, z, w]`
/// ordering used by Bevy and Web APIs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraOrientationReadModel {
    pub rotation_xyzw: [f32; 4],
}

impl CameraOrientationReadModel {
    pub const IDENTITY: Self = Self {
        rotation_xyzw: [0.0, 0.0, 0.0, 1.0],
    };

    pub fn from_rotation_xyzw(rotation: [f32; 4]) -> Option<Self> {
        if !rotation.iter().all(|value| value.is_finite()) {
            return None;
        }
        let length = rotation
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();
        if !length.is_finite() || length <= f32::EPSILON {
            return None;
        }
        Some(Self {
            rotation_xyzw: rotation.map(|value| value / length),
        })
    }

    pub fn is_finite(&self) -> bool {
        self.rotation_xyzw.iter().all(|value| value.is_finite())
    }
}

impl Default for CameraOrientationReadModel {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// The six canonical camera directions exposed by the viewport ViewCube.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardView {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
}

impl Default for StandardView {
    fn default() -> Self {
        Self::Front
    }
}
