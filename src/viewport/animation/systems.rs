//! Viewport playback clock bridged to `usd_bevy::StageTime`.

use bevy::prelude::*;
use usd_bevy::{
    AnimatedPrims, LiveStage, ProgressiveProjectionState, ProjectionReadiness, StageTime,
};
use viewport_protocol::{TimelineReadModel, ViewportEvent, ViewportEventEnvelope};

use super::UsdStageTime;
use crate::viewport::api::ViewportEventOutbox;

/// Advance the viewport clock and publish its current USD time code to the
/// live route system. `LiveStagePlugin` resamples animated routes when this
/// resource changes; the viewport no longer owns a second animation evaluator.
pub(crate) fn tick_stage_time(
    time: Res<Time>,
    mut clock: ResMut<UsdStageTime>,
    mut stage_time: ResMut<StageTime>,
    stage: Option<NonSend<LiveStage>>,
    animated: Res<AnimatedPrims>,
    projection: Option<Res<ProgressiveProjectionState>>,
    mut outbox: Option<ResMut<ViewportEventOutbox>>,
) {
    let stage_identity = stage.as_ref().map(|stage| stage.stage_identity());
    if clock.stage_identity() != stage_identity {
        clock.reset_for_stage(stage_identity);
    }
    let Some(stage) = stage else {
        stage_time.current = 0.0;
        return;
    };
    if projection
        .as_ref()
        .is_some_and(|state| state.readiness() != ProjectionReadiness::Ready)
    {
        return;
    }
    let initialized_now = if !clock.initialized {
        clock.start_time_code = stage.stage.start_time_code();
        clock.end_time_code = stage.stage.end_time_code();
        clock.time_codes_per_second = stage.stage.time_codes_per_second().max(1.0);
        let has_animation = !animated.0.is_empty();
        if !has_animation {
            clock.end_time_code = clock.start_time_code;
        }
        clock.playing = has_animation;
        clock.initialized = true;
        true
    } else {
        false
    };

    if clock.playing {
        clock.seconds += time.delta_secs_f64();
        let duration = clock.duration_seconds();
        if duration > 0.0 && clock.seconds >= duration {
            clock.seconds = clock.seconds.rem_euclid(duration);
        }
    }
    stage_time.current = clock.current_time_code();

    if initialized_now && let Some(outbox) = outbox.as_deref_mut() {
        outbox.push(ViewportEventEnvelope::new(
            None,
            ViewportEvent::TimelineChanged {
                timeline: TimelineReadModel {
                    seconds: clock.seconds,
                    playing: clock.playing,
                    start_time_code: clock.start_time_code,
                    end_time_code: clock.end_time_code,
                    time_codes_per_second: clock.time_codes_per_second,
                },
            },
        ));
    }
}
