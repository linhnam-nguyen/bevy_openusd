//! Animation playback state and systems.

mod state;
mod systems;

use bevy::app::{App, Update};
use bevy::ecs::schedule::IntoScheduleConfigs;
use usd_bevy::LiveStageSet;

use crate::viewport::api::ViewportBridgeSet;

pub(crate) use state::UsdStageTime;

pub(crate) fn configure(app: &mut App) {
    app.add_systems(
        Update,
        (
            systems::tick_stage_time
                .after(LiveStageSet::Reconcile)
                .before(LiveStageSet::Animation)
                .before(ViewportBridgeSet::PublishStageLoadState),
            systems::publish_timeline_state
                .after(ViewportBridgeSet::PublishStageLoadState)
                .before(ViewportBridgeSet::ReduceEvents),
        ),
    );
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/animation/tests.rs"]
pub(in crate::viewport) mod tests;
