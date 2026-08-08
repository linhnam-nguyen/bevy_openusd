//! Animation playback state and systems.

mod state;
mod systems;

pub(crate) use state::{PendingAnimationClip, UsdStageTime};
pub(crate) use systems::{
    apply_live_animation_clip, drive_blend_shape_weights, drive_skel_animations,
    evaluate_animated_prims, tick_stage_time,
};
