use viewport_protocol::{GroundGridOrigin, OverlayKind, ViewportEvent, ViewportEventEnvelope};

use super::super::helpers::{emit_presentation_changed, set_overlay};
use crate::viewport::api::ViewportEventOutbox;
use crate::viewport::physics::PhysicsActive;
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::session::LoaderTuning;

pub(super) fn set_overlay_command(
    request_id: String,
    overlay: OverlayKind,
    enabled: bool,
    outbox: &mut ViewportEventOutbox,
    toggles: &mut DisplayToggles,
    tuning: &LoaderTuning,
) {
    set_overlay(toggles, overlay, enabled);
    emit_presentation_changed(outbox, request_id, toggles, tuning);
}

pub(super) fn set_grid_origin(
    request_id: String,
    origin: GroundGridOrigin,
    outbox: &mut ViewportEventOutbox,
    toggles: &mut DisplayToggles,
    tuning: &LoaderTuning,
) {
    toggles.ground_grid_origin = origin;
    emit_presentation_changed(outbox, request_id, toggles, tuning);
}

pub(super) fn set_prim_marker_bias(
    request_id: String,
    bias: f32,
    outbox: &mut ViewportEventOutbox,
    toggles: &mut DisplayToggles,
    tuning: &LoaderTuning,
) {
    toggles.prim_marker_bias = bias.clamp(0.0, 5.0);
    emit_presentation_changed(outbox, request_id, toggles, tuning);
}

pub(super) fn set_light_intensity(
    request_id: String,
    scale: f32,
    outbox: &mut ViewportEventOutbox,
    toggles: &mut DisplayToggles,
    tuning: &LoaderTuning,
) {
    toggles.light_intensity_scale = scale.clamp(0.0, 5.0);
    emit_presentation_changed(outbox, request_id, toggles, tuning);
}

pub(super) fn set_curve_tuning(
    request_id: String,
    next: viewport_protocol::CurveTuning,
    outbox: &mut ViewportEventOutbox,
    toggles: &mut DisplayToggles,
    tuning: &mut LoaderTuning,
) {
    tuning.curves.default_radius = next.default_radius.clamp(0.001, 0.2);
    tuning.curves.ring_segments = next.ring_segments.clamp(3, 24);
    tuning.curves.point_scale = next.point_scale.clamp(0.05, 4.0);
    emit_presentation_changed(outbox, request_id, toggles, tuning);
}

pub(super) fn set_physics(
    request_id: String,
    running: bool,
    outbox: &mut ViewportEventOutbox,
    physics: &mut PhysicsActive,
) {
    physics.0 = running;
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::PhysicsChanged { running },
    ));
}
