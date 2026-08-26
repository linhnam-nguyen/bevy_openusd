//! Pose authority for the renderer-only aggregate Section Box.

use bevy::prelude::{Transform, Vec3};

use super::{SectionBoxBounds, SectionBoxClipPlanes};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SectionBoxPoseAuthority {
    #[default]
    AutoFit,
    UserAdjusted,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct SectionBoxPose {
    pub(super) transform: Transform,
    pub(super) clip_planes: SectionBoxClipPlanes,
    pub(super) authority: SectionBoxPoseAuthority,
}

pub(super) fn next_section_box_pose(
    current_transform: Transform,
    current_clip_planes: SectionBoxClipPlanes,
    current_authority: SectionBoxPoseAuthority,
    visible: bool,
    bounds: Option<SectionBoxBounds>,
    force_auto_fit: bool,
) -> SectionBoxPose {
    let Some(bounds) = bounds.filter(|_| visible) else {
        return SectionBoxPose {
            transform: Transform::IDENTITY,
            clip_planes: SectionBoxClipPlanes::default(),
            authority: SectionBoxPoseAuthority::AutoFit,
        };
    };

    let authority = if force_auto_fit || current_authority == SectionBoxPoseAuthority::AutoFit {
        SectionBoxPoseAuthority::AutoFit
    } else {
        SectionBoxPoseAuthority::UserAdjusted
    };
    if authority == SectionBoxPoseAuthority::UserAdjusted {
        return SectionBoxPose {
            transform: current_transform,
            clip_planes: current_clip_planes,
            authority,
        };
    }

    let transform = fit_transform(bounds);
    SectionBoxPose {
        transform,
        clip_planes: SectionBoxClipPlanes::from_bounds(bounds),
        authority,
    }
}

pub(super) fn fit_transform(bounds: SectionBoxBounds) -> Transform {
    Transform {
        translation: (bounds.min + bounds.max) * 0.5,
        scale: (bounds.max - bounds.min).max(Vec3::splat(0.0001)),
        ..Transform::IDENTITY
    }
}
