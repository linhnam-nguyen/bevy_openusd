//! Viewport playback clock bridged to `usd_bevy::StageTime`.

use bevy::prelude::*;
use usd_bevy::{AnimatedPrims, LiveStage, StageTime};

use super::UsdStageTime;

/// Advance the viewport clock and publish its current USD time code to the
/// live route system. `LiveStagePlugin` resamples animated routes when this
/// resource changes; the viewport no longer owns a second animation evaluator.
pub(crate) fn tick_stage_time(
    time: Res<Time>,
    mut clock: ResMut<UsdStageTime>,
    mut stage_time: ResMut<StageTime>,
    stage: Option<NonSend<LiveStage>>,
    animated: Res<AnimatedPrims>,
) {
    if !clock.initialized {
        let Some(stage) = stage else {
            return;
        };
        clock.start_time_code = stage.stage.start_time_code();
        clock.end_time_code = stage.stage.end_time_code();
        clock.time_codes_per_second = stage.stage.time_codes_per_second().max(1.0);
        let has_animation = !animated.0.is_empty();
        if !has_animation {
            clock.end_time_code = clock.start_time_code;
        }
        clock.playing = has_animation;
        clock.initialized = true;
    }

    let benchmark_paused = std::env::var_os("USDHUB_BENCHMARK_PAUSED").is_some();
    let benchmark_playing = std::env::var_os("USDHUB_BENCHMARK_PLAYING").is_some();
    if benchmark_paused {
        clock.playing = false;
    } else if benchmark_playing {
        clock.playing = true;
    }
    if clock.playing {
        clock.seconds += if benchmark_playing {
            1.0 / 60.0
        } else {
            time.delta_secs_f64()
        };
        let duration = clock.duration_seconds();
        if duration > 0.0 && clock.seconds >= duration {
            clock.seconds = clock.seconds.rem_euclid(duration);
        }
    }
    stage_time.current = clock.current_time_code();
}
