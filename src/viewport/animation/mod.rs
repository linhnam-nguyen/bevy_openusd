//! Animation playback state and systems.

mod state;
mod systems;

pub(crate) use state::UsdStageTime;
pub(crate) use systems::tick_stage_time;
