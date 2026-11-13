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
#[path = "../../../tests/unit/viewport/app/cadence_inline_tests.rs"]
mod tests;
