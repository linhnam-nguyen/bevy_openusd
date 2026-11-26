use viewport_protocol::{TimelineReadModel, ViewportEvent, ViewportEventEnvelope};

use super::super::helpers::timeline_read_model;
use crate::animation::{PlaybackDomainReadiness, UsdStageTime};
use crate::api::ViewportEventOutbox;

pub(super) fn set_playback(
    request_id: String,
    playing: bool,
    outbox: &mut ViewportEventOutbox,
    clock: &mut UsdStageTime,
) {
    if playing && clock.domain_readiness != PlaybackDomainReadiness::Ready {
        super::super::helpers::reject(outbox, request_id,
            format!("playback unavailable: {:?}", clock.domain_readiness));
        return;
    }
    clock.playing = playing;
    emit_timeline_changed(request_id, outbox, clock);
}

pub(super) fn seek(
    request_id: String,
    seconds: f64,
    outbox: &mut ViewportEventOutbox,
    clock: &mut UsdStageTime,
) {
    clock.seconds = seconds.clamp(0.0, clock.duration_seconds());
    emit_timeline_changed(request_id, outbox, clock);
}

fn emit_timeline_changed(
    request_id: String,
    outbox: &mut ViewportEventOutbox,
    clock: &UsdStageTime,
) {
    let timeline: TimelineReadModel = timeline_read_model(clock);
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::TimelineChanged { timeline },
    ));
}
