//! Diagnostic resources and viewer-facing log capture.

mod dumps;
pub(crate) mod log_capture;

pub(crate) use dumps::{
    debug_dump_layout_once, debug_dump_physics_once, debug_dump_physics_tick,
    debug_origin_prims_once,
};
