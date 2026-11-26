//! Viewport playback clock bridged to `usd_bevy::StageTime`.

use bevy::prelude::*;
use usd_bevy::{
    AnimatedPrims, LiveStage, ProgressiveProjectionState, ProjectionReadiness, StageTime,
};
use viewport_protocol::{TimelineReadModel, ViewportEvent, ViewportEventEnvelope};

use super::UsdStageTime;
use crate::api::ViewportEventOutbox;

const TIMELINE_EVENT_INTERVAL_SECONDS: f64 = 0.1;

#[derive(Default)]
pub(crate) struct TimelinePublicationState {
    elapsed_since_publish: f64,
    last: Option<TimelineReadModel>,
}

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
    let has_animated_prims = !animated.0.is_empty();
    let observed = (
        stage.stage.start_time_code().to_bits(),
        stage.stage.end_time_code().to_bits(),
        stage.stage.time_codes_per_second().to_bits(),
        has_animated_prims,
        stage.authoring_generation().0,
    );
    if !clock.initialized || clock.observed_stage_range() != Some(observed) {
        let was_initialized = clock.initialized;
        let was_playable = clock.duration_seconds().is_finite() && clock.duration_seconds() > 0.0;
        let domain = crate::document::stages::metadata::inspect_stage_time_metadata(&stage.stage, true);
        match domain {
            Ok(metadata) => {
                let (start, end) = metadata.range.map_or((0.0, 0.0), |range| (range.start, range.end));
                clock.start_time_code = start;
                clock.end_time_code = end;
                clock.time_codes_per_second = metadata.time_codes_per_second;
                let duration = clock.duration_seconds();
                let playable = has_animated_prims && duration.is_finite() && duration > 0.0;
                clock.domain_readiness = if playable {
                    super::state::PlaybackDomainReadiness::Ready
                } else {
                    super::state::PlaybackDomainReadiness::NoPlayableAnimation
                };
                if !playable {
                    clock.end_time_code = start;
                    clock.seconds = 0.0;
                    clock.playing = false;
                } else if !was_initialized || !was_playable {
                    clock.seconds = 0.0;
                    clock.playing = true;
                } else {
                    clock.seconds = if clock.seconds.is_finite() { clock.seconds.clamp(0.0, duration) } else { 0.0 };
                }
            }
            Err(error) => {
                log::warn!("USD playback domain unavailable: {error:#}");
                clock.domain_readiness = super::state::PlaybackDomainReadiness::Unavailable;
                clock.start_time_code = 0.0;
                clock.end_time_code = 0.0;
                clock.time_codes_per_second = 24.0;
                clock.seconds = 0.0;
                clock.playing = false;
            }
        }
        clock.initialized = true;
        clock.set_observed_stage_range(observed);
    }

    if clock.domain_readiness != super::state::PlaybackDomainReadiness::Ready {
        clock.playing = false;
    }
    advance_looping_playback(&mut clock, time.delta_secs_f64());
    stage_time.current = clock.current_time_code();
}

fn advance_looping_playback(clock: &mut UsdStageTime, delta_seconds: f64) {
    if !clock.playing || !delta_seconds.is_finite() {
        return;
    }

    let duration = clock.duration_seconds();
    if !duration.is_finite() || duration <= 0.0 {
        return;
    }

    let current = if clock.seconds.is_finite() {
        clock.seconds
    } else {
        0.0
    };
    clock.seconds = (current + delta_seconds).rem_euclid(duration);
}

/// Publishes authoritative timeline progress after the Ready snapshot.
///
/// The rendered animation remains entirely server-side. This event only keeps
/// the remote playhead and Play/Pause label synchronized without sending one
/// reliable DataChannel message per render frame.
pub(crate) fn publish_timeline_state(
    time: Res<Time>,
    clock: Res<UsdStageTime>,
    mut publication: Local<TimelinePublicationState>,
    mut outbox: ResMut<ViewportEventOutbox>,
) {
    if !clock.initialized {
        publication.elapsed_since_publish = 0.0;
        publication.last = None;
        return;
    }

    publication.elapsed_since_publish += time.delta_secs_f64().max(0.0);
    let timeline = TimelineReadModel {
        seconds: clock.seconds,
        playing: clock.playing,
        start_time_code: clock.start_time_code,
        end_time_code: clock.end_time_code,
        time_codes_per_second: clock.time_codes_per_second,
    };
    let state_changed = publication.last.as_ref().is_none_or(|last| {
        last.playing != timeline.playing
            || last.start_time_code != timeline.start_time_code
            || last.end_time_code != timeline.end_time_code
            || last.time_codes_per_second != timeline.time_codes_per_second
    });
    let wrapped = publication
        .last
        .as_ref()
        .is_some_and(|last| timeline.playing && timeline.seconds < last.seconds);
    let periodic_update =
        clock.playing && publication.elapsed_since_publish >= TIMELINE_EVENT_INTERVAL_SECONDS;
    if !state_changed && !wrapped && !periodic_update {
        return;
    }

    outbox.push(ViewportEventEnvelope::new(
        None,
        ViewportEvent::TimelineChanged {
            timeline: timeline.clone(),
        },
    ));
    publication.elapsed_since_publish = 0.0;
    publication.last = Some(timeline);
}
