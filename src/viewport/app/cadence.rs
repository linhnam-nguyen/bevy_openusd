//! Runtime authority for headless renderer cadence.
//!
//! The renderer cadence is deliberately separate from WebRTC encoder caps.
//! The headless app runner reads the effective target after each Bevy update,
//! so an accepted FPS change affects the next render interval rather than
//! merely changing metadata on an encoder.

use std::time::{Duration, Instant};

use bevy::app::{App, AppExit, PluginsState};

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingCadence {
    fps: Option<u32>,
    generation: u64,
    request_id: Option<String>,
}

/// Bevy-main-thread state for requested and applied renderer cadence.
#[derive(bevy::prelude::Resource, Debug, Default)]
pub(crate) struct RendererCadence {
    requested_fps: Option<u32>,
    effective_renderer_target_fps: Option<u32>,
    latest_stream_generation: u64,
    local_generation: u64,
    applied_generation: u64,
    effective_encoded_fps: Option<u32>,
    pending_stream: Option<PendingCadence>,
    pending_local: Option<PendingCadence>,
}

/// Result of applying a pending cadence request.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AppliedCadence {
    pub(crate) fps: Option<u32>,
    pub(crate) generation: u64,
    pub(crate) changed: bool,
    pub(crate) request_id: Option<String>,
}

impl RendererCadence {
    pub(crate) fn new(initial_fps: Option<u32>) -> Self {
        Self {
            requested_fps: initial_fps,
            effective_renderer_target_fps: initial_fps,
            ..Self::default()
        }
    }

    pub(crate) fn requested_fps(&self) -> Option<u32> {
        self.requested_fps
    }

    pub(crate) fn effective_renderer_target_fps(&self) -> Option<u32> {
        self.effective_renderer_target_fps
    }

    pub(crate) fn applied_generation(&self) -> u64 {
        self.applied_generation
    }

    pub(crate) fn effective_encoded_fps(&self) -> Option<u32> {
        self.effective_encoded_fps
    }

    /// Queues a renderer command without discarding an accepted stream update.
    /// A local request has priority when both sources request a different
    /// target; the stream request remains queued for the next application.
    pub(crate) fn request_local(&mut self, fps: Option<u32>, request_id: String) -> bool {
        self.local_generation = self.local_generation.saturating_add(1);
        self.requested_fps = fps;
        if self.effective_renderer_target_fps == fps {
            return false;
        }

        self.pending_local = Some(PendingCadence {
            fps,
            generation: self.local_generation,
            request_id: Some(request_id),
        });
        true
    }

    /// Queues a newer stream-generation cadence request. A stale request is
    /// rejected before it can affect either the effective target or runner.
    pub(crate) fn request_stream(&mut self, fps: Option<u32>, generation: u64) -> bool {
        if generation <= self.latest_stream_generation {
            return false;
        }
        self.latest_stream_generation = generation;
        self.requested_fps = fps;
        self.pending_stream = Some(PendingCadence {
            fps,
            generation,
            request_id: None,
        });
        true
    }

    pub(crate) fn apply_pending(&mut self) -> Option<AppliedCadence> {
        // A typed renderer command is the explicit local authority for its
        // own request. A stream request is never overwritten: if local wins,
        // the accepted stream cadence remains queued and is applied next.
        let pending = self
            .pending_local
            .take()
            .or_else(|| self.pending_stream.take())?;
        let changed = self.effective_renderer_target_fps != pending.fps;
        self.effective_renderer_target_fps = pending.fps;
        self.applied_generation = pending.generation;
        if pending.request_id.is_none() {
            // The streaming router uses 60 FPS for an uncapped transport
            // request; expose that encoder target separately from renderer
            // cadence rather than conflating the two values.
            self.effective_encoded_fps = Some(pending.fps.unwrap_or(60));
        }
        Some(AppliedCadence {
            fps: pending.fps,
            generation: pending.generation,
            changed,
            request_id: pending.request_id,
        })
    }

    pub(crate) fn wait_duration(&self) -> Option<Duration> {
        self.effective_renderer_target_fps
            .map(|fps| Duration::from_secs_f64(1.0 / f64::from(fps)))
    }
}

/// Runs a headless Bevy app with the currently effective renderer cadence.
pub(crate) fn run_headless(mut app: App) -> AppExit {
    let plugins_state = app.plugins_state();
    if plugins_state != PluginsState::Cleaned {
        while app.plugins_state() == PluginsState::Adding {
            std::thread::yield_now();
        }
        app.finish();
        app.cleanup();
    }

    loop {
        let start = Instant::now();
        app.update();

        if let Some(exit) = app.should_exit() {
            return exit;
        }

        let elapsed = start.elapsed();
        if let Some(wait) = app
            .world()
            .get_resource::<RendererCadence>()
            .and_then(RendererCadence::wait_duration)
            && elapsed < wait
        {
            std::thread::sleep(wait - elapsed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_publishes_initial_requested_and_effective_target() {
        let cadence = RendererCadence::new(Some(60));

        assert_eq!(cadence.requested_fps(), Some(60));
        assert_eq!(cadence.effective_renderer_target_fps(), Some(60));
        assert_eq!(cadence.applied_generation(), 0);
        assert_eq!(
            cadence.wait_duration(),
            Some(Duration::from_secs_f64(1.0 / 60.0))
        );
    }

    #[test]
    fn cadence_transitions_30_to_60_to_120_only_after_application() {
        let mut cadence = RendererCadence::new(Some(60));

        assert!(cadence.request_local(Some(30), "fps-30".to_owned()));
        assert_eq!(cadence.effective_renderer_target_fps(), Some(60));
        assert_eq!(cadence.apply_pending().unwrap().fps, Some(30));
        assert_eq!(cadence.effective_renderer_target_fps(), Some(30));

        assert!(cadence.request_local(Some(60), "fps-60".to_owned()));
        assert_eq!(cadence.effective_renderer_target_fps(), Some(30));
        assert_eq!(cadence.apply_pending().unwrap().fps, Some(60));

        assert!(cadence.request_local(Some(120), "fps-120".to_owned()));
        assert_eq!(cadence.effective_renderer_target_fps(), Some(60));
        assert_eq!(cadence.apply_pending().unwrap().fps, Some(120));
        assert_eq!(
            cadence.wait_duration(),
            Some(Duration::from_secs_f64(1.0 / 120.0))
        );
    }

    #[test]
    fn stale_stream_generation_cannot_replace_a_newer_request() {
        let mut cadence = RendererCadence::new(Some(60));

        assert!(cadence.request_stream(Some(30), 2));
        assert!(!cadence.request_stream(Some(120), 1));
        assert_eq!(cadence.requested_fps(), Some(30));
        assert_eq!(cadence.apply_pending().unwrap().fps, Some(30));
    }

    #[test]
    fn accepted_stream_fps_survives_a_same_frame_local_request() {
        let mut cadence = RendererCadence::new(Some(60));

        assert!(cadence.request_stream(Some(120), 2));
        assert!(!cadence.request_local(Some(60), "presentation-1".to_owned()));

        let applied = cadence
            .apply_pending()
            .expect("stream cadence must remain queued");
        assert_eq!(applied.fps, Some(120));
        assert_eq!(applied.request_id, None);
        assert_eq!(cadence.effective_renderer_target_fps(), Some(120));
    }

    #[test]
    fn local_fps_has_explicit_priority_but_does_not_discard_stream_fps() {
        let mut cadence = RendererCadence::new(Some(60));

        assert!(cadence.request_stream(Some(120), 2));
        assert!(cadence.request_local(Some(30), "presentation-1".to_owned()));

        assert_eq!(cadence.apply_pending().unwrap().fps, Some(30));
        assert_eq!(cadence.apply_pending().unwrap().fps, Some(120));
    }

    #[test]
    fn fps_only_stream_update_does_not_own_dimensions() {
        let mut cadence = RendererCadence::new(Some(30));
        assert!(cadence.request_stream(Some(120), 3));
        let applied = cadence.apply_pending().unwrap();

        assert_eq!(applied.fps, Some(120));
        assert_eq!(cadence.effective_renderer_target_fps(), Some(120));
        // Dimensions are intentionally absent from this resource: stream
        // resize ownership remains with OffscreenTarget.
    }

    #[test]
    fn effective_state_is_not_published_until_pending_request_is_applied() {
        let mut cadence = RendererCadence::new(Some(60));
        assert!(cadence.request_local(None, "uncapped".to_owned()));

        assert_eq!(cadence.effective_renderer_target_fps(), Some(60));
        let applied = cadence.apply_pending().unwrap();
        assert!(applied.changed);
        assert_eq!(applied.request_id.as_deref(), Some("uncapped"));
        assert_eq!(cadence.effective_renderer_target_fps(), None);
    }
}
