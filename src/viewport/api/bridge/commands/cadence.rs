use bevy::prelude::*;
use viewport_protocol::{ViewportEvent, ViewportEventEnvelope};

use super::super::helpers::presentation_read_model;
use crate::viewport::api::ViewportEventOutbox;
use crate::viewport::app::cadence::RendererCadence;
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::session::LoaderTuning;

/// Publishes the correlated presentation event only after the requested FPS
/// has become the effective target consumed by the headless runner.
pub(crate) fn apply_pending_renderer_cadence(
    mut cadence: Option<ResMut<RendererCadence>>,
    mut toggles: ResMut<DisplayToggles>,
    mut outbox: ResMut<ViewportEventOutbox>,
    tuning: Res<LoaderTuning>,
) {
    let Some(cadence) = cadence.as_deref_mut() else {
        return;
    };
    let Some(applied) = cadence.apply_pending() else {
        return;
    };

    toggles.renderer.preferred_fps = applied.fps;
    if applied.changed || applied.request_id.is_some() {
        outbox.push(ViewportEventEnvelope::new(
            applied.request_id,
            ViewportEvent::PresentationChanged {
                presentation: presentation_read_model(&toggles, &tuning),
            },
        ));
    }
}
