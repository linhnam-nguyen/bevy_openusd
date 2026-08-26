use bevy::prelude::*;
use bevy_glacial::prelude::BoundsFace;

/// Explicit identity of one renderer-owned Section Box face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SectionBoxFace {
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}

impl SectionBoxFace {
    pub(crate) const ALL: [Self; 6] = [
        Self::PositiveX,
        Self::NegativeX,
        Self::PositiveY,
        Self::NegativeY,
        Self::PositiveZ,
        Self::NegativeZ,
    ];

    pub(crate) fn from_bounds_face(face: BoundsFace) -> Self {
        match face {
            BoundsFace::PositiveX => Self::PositiveX,
            BoundsFace::NegativeX => Self::NegativeX,
            BoundsFace::PositiveY => Self::PositiveY,
            BoundsFace::NegativeY => Self::NegativeY,
            BoundsFace::PositiveZ => Self::PositiveZ,
            BoundsFace::NegativeZ => Self::NegativeZ,
        }
    }

    pub(crate) const fn axis_index(self) -> usize {
        match self {
            Self::PositiveX | Self::NegativeX => 0,
            Self::PositiveY | Self::NegativeY => 1,
            Self::PositiveZ | Self::NegativeZ => 2,
        }
    }

    pub(crate) const fn local_axis(self) -> Vec3 {
        match self {
            Self::PositiveX | Self::NegativeX => Vec3::X,
            Self::PositiveY | Self::NegativeY => Vec3::Y,
            Self::PositiveZ | Self::NegativeZ => Vec3::Z,
        }
    }

    pub(crate) const fn sign(self) -> f32 {
        match self {
            Self::PositiveX | Self::PositiveY | Self::PositiveZ => 1.0,
            Self::NegativeX | Self::NegativeY | Self::NegativeZ => -1.0,
        }
    }

    pub(crate) const fn opposite(self) -> Self {
        match self {
            Self::PositiveX => Self::NegativeX,
            Self::NegativeX => Self::PositiveX,
            Self::PositiveY => Self::NegativeY,
            Self::NegativeY => Self::PositiveY,
            Self::PositiveZ => Self::NegativeZ,
            Self::NegativeZ => Self::PositiveZ,
        }
    }
}

/// Renderer-safe minimum dimension for a Section Box.
pub(crate) const MIN_SECTION_BOX_THICKNESS: f32 = 0.001;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SectionBoxFaceDrag {
    pub(crate) face: SectionBoxFace,
    pub(crate) start_transform: Transform,
}

/// Applies a signed one-face displacement in Section Box local coordinates.
///
/// Positive displacement extends the selected face outward. The opposite
/// face remains fixed, including for rotated Section Boxes.
pub(crate) fn resize_section_box_face(
    transform: Transform,
    face: SectionBoxFace,
    displacement: f32,
) -> Transform {
    let mut size = transform.scale.abs();
    let axis = face.axis_index();
    let old_size = size[axis];
    let new_size = (old_size + displacement).max(MIN_SECTION_BOX_THICKNESS);
    let effective_delta = new_size - old_size;
    size[axis] = new_size;

    let normal_world = transform.rotation * (face.local_axis() * face.sign());
    Transform {
        translation: transform.translation + normal_world * (effective_delta * 0.5),
        rotation: transform.rotation,
        scale: size,
    }
}

#[cfg(test)]
#[path = "section_box_face_tests.rs"]
mod tests;
