use log::{debug, warn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use viewport_protocol::{ActiveStreamConfiguration, CodecId, ViewportMetrics};

use crate::VideoFrame;
use crate::encode::EncodePipeline;

pub(super) struct ActiveFrameTarget {
    connection_id: u64,
    encoder: Arc<EncodePipeline>,
    codec: CodecId,
    expected: Option<ViewportMetrics>,
    current: Option<ActiveStreamConfiguration>,
    applied: Option<ActiveStreamConfiguration>,
}

#[derive(Default)]
pub(super) struct FrameRouterState {
    targets: HashMap<u64, ActiveFrameTarget>,
    expected: Option<ViewportMetrics>,
    current: Option<ActiveStreamConfiguration>,
}

/// Routes each raw frame to every admitted encoder without exposing the
/// encoders or GStreamer objects to Bevy.
#[derive(Clone, Default)]
pub(crate) struct FrameRouter {
    state: Arc<Mutex<FrameRouterState>>,
}

impl FrameRouter {
    pub(super) fn activate(
        &self,
        connection_id: u64,
        encoder: Arc<EncodePipeline>,
        codec: CodecId,
    ) {
        if let Ok(mut state) = self.state.lock() {
            let expected = state.expected.clone();
            let current = expected.is_none().then(|| state.current.clone()).flatten();
            state.targets.insert(
                connection_id,
                ActiveFrameTarget {
                    connection_id,
                    encoder,
                    codec,
                    expected,
                    current,
                    applied: None,
                },
            );
        }
    }

    pub(super) fn deactivate(&self, connection_id: u64) {
        if let Ok(mut state) = self.state.lock() {
            state.targets.remove(&connection_id);
            if state.targets.is_empty() {
                state.expected = None;
                state.current = None;
            }
        }
    }

    pub(super) fn configure(&self, connection_id: u64, metrics: ViewportMetrics) {
        if let Ok(mut state) = self.state.lock() {
            if !state.targets.contains_key(&connection_id) {
                return;
            }
            let newest_generation = state
                .expected
                .as_ref()
                .map(|expected| expected.generation)
                .into_iter()
                .chain(state.current.as_ref().map(|current| current.generation))
                .max()
                .unwrap_or(0);
            if metrics.generation <= newest_generation {
                warn!(
                    "[viewport-frame-pump] ignored stale stream configuration generation {} (newest {})",
                    metrics.generation, newest_generation
                );
                return;
            }
            state.expected = Some(metrics.clone());
            state.current = None;
            for target in state.targets.values_mut() {
                target.expected = Some(metrics.clone());
                target.current = None;
                target.applied = None;
            }
        }
    }

    pub(super) fn take_applied(&self, connection_id: u64) -> Option<ActiveStreamConfiguration> {
        self.state
            .lock()
            .ok()?
            .targets
            .get_mut(&connection_id)?
            .applied
            .take()
    }

    pub(super) fn current_metrics(&self) -> Option<ViewportMetrics> {
        let state = self.state.lock().ok()?;
        state.expected.clone().or_else(|| {
            state.current.as_ref().map(|current| ViewportMetrics {
                css_width: current.width,
                css_height: current.height,
                device_pixel_ratio: 1.0,
                requested_width: current.width,
                requested_height: current.height,
                preferred_fps: Some(current.fps),
                generation: current.generation,
            })
        })
    }

    pub(super) fn push(&self, frame: &VideoFrame) {
        let targets = self
            .state
            .lock()
            .ok()
            .map(|state| {
                state
                    .targets
                    .values()
                    .map(|target| {
                        (
                            target.connection_id,
                            Arc::clone(&target.encoder),
                            target.codec,
                            target.expected.clone(),
                            target.current.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        for (connection_id, encoder, codec, expected, current) in targets {
            if let Some(expected_metrics) = expected {
                if frame.width != expected_metrics.requested_width
                    || frame.height != expected_metrics.requested_height
                    || frame.generation != expected_metrics.generation
                {
                    continue;
                }

                let fps = expected_metrics.preferred_fps.unwrap_or(60);
                if let Err(error) = encoder.set_video_caps(frame.width, frame.height, fps) {
                    warn!("[viewport-frame-pump] stream caps update failed: {error:?}");
                    continue;
                }
                if let Err(error) = encoder.request_sync_frame_after_caps_change() {
                    warn!(
                        "[viewport-frame-pump] sync-frame/configuration refresh failed: {error:?}"
                    );
                    continue;
                }
                if let Err(error) = encoder.push_rgba_frame(&frame.rgba) {
                    debug!("[viewport-frame-pump] frame push failed: {error:?}");
                    continue;
                }

                let configuration = ActiveStreamConfiguration {
                    width: frame.width,
                    height: frame.height,
                    fps,
                    codec,
                    generation: frame.generation,
                };
                if let Ok(mut state) = self.state.lock()
                    && let Some(target) = state.targets.get_mut(&connection_id)
                    && target
                        .expected
                        .as_ref()
                        .is_some_and(|current| current == &expected_metrics)
                {
                    target.expected = None;
                    target.current = Some(configuration.clone());
                    target.applied = Some(configuration.clone());
                    let all_configured = state.targets.values().all(|target| {
                        target
                            .current
                            .as_ref()
                            .is_some_and(|current| current == &configuration)
                    });
                    if all_configured {
                        state.current = Some(configuration);
                        state.expected = None;
                    }
                }
                continue;
            }

            let Some(current) = current else {
                continue;
            };
            if frame.width != current.width
                || frame.height != current.height
                || frame.generation != current.generation
            {
                continue;
            }
            if let Err(error) = encoder.push_rgba_frame(&frame.rgba) {
                debug!("[viewport-frame-pump] frame push failed: {error:?}");
            }
        }
    }
}
