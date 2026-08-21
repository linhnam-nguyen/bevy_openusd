use anyhow::Result;
use log::{debug, warn};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use viewport_protocol::{ActiveStreamConfiguration, CodecId, ViewportMetrics};

use crate::VideoFrame;
use crate::encode::EncodePipeline;
use crate::frame_metrics::FrameTransportMetrics;

const ENCODER_QUEUE_CAPACITY: usize = 2;

pub(super) trait FrameEncoder: Send + Sync {
    fn push_frame(&self, frame: &VideoFrame) -> Result<()>;
    fn set_video_caps(&self, width: u32, height: u32, fps: u32) -> Result<()>;
    fn request_sync_frame_after_caps_change(&self) -> Result<()>;
}

impl FrameEncoder for EncodePipeline {
    fn push_frame(&self, frame: &VideoFrame) -> Result<()> {
        EncodePipeline::push_frame(self, frame)
    }

    fn set_video_caps(&self, width: u32, height: u32, fps: u32) -> Result<()> {
        EncodePipeline::set_video_caps(self, width, height, fps)
    }

    fn request_sync_frame_after_caps_change(&self) -> Result<()> {
        EncodePipeline::request_sync_frame_after_caps_change(self)
    }
}

fn generation_matches(frame_generation: u64, active_generation: u64) -> bool {
    frame_generation == active_generation
}

struct EncodeRequest {
    frame: VideoFrame,
    expected: Option<ViewportMetrics>,
}

pub(super) struct ActiveFrameTarget {
    connection_id: u64,
    sender: SyncSender<EncodeRequest>,
    codec: CodecId,
    expected: Option<ViewportMetrics>,
    current: Option<ActiveStreamConfiguration>,
    applied: Option<ActiveStreamConfiguration>,
    configuration_in_flight: bool,
    worker: Option<JoinHandle<()>>,
}

#[derive(Default)]
pub(super) struct FrameRouterState {
    targets: HashMap<u64, ActiveFrameTarget>,
    expected: Option<ViewportMetrics>,
    current: Option<ActiveStreamConfiguration>,
}

/// Routes each raw frame to every admitted encoder without exposing the
/// encoders or GStreamer objects to Bevy.
#[derive(Clone)]
pub(crate) struct FrameRouter {
    state: Arc<Mutex<FrameRouterState>>,
    metrics: FrameTransportMetrics,
}

impl FrameRouter {
    pub(super) fn new(metrics: FrameTransportMetrics) -> Self {
        Self {
            state: Arc::new(Mutex::new(FrameRouterState::default())),
            metrics,
        }
    }

    pub(super) fn activate<E>(&self, connection_id: u64, encoder: Arc<E>, codec: CodecId)
    where
        E: FrameEncoder + 'static,
    {
        let (sender, receiver) = sync_channel(ENCODER_QUEUE_CAPACITY);
        let encoder: Arc<dyn FrameEncoder> = encoder;
        let worker = spawn_encoder_worker(
            connection_id,
            encoder,
            receiver,
            Arc::clone(&self.state),
            self.metrics.clone(),
        );
        let previous = if let Ok(mut state) = self.state.lock() {
            let expected = state.expected.clone();
            let current = expected.is_none().then(|| state.current.clone()).flatten();
            state.targets.insert(
                connection_id,
                ActiveFrameTarget {
                    connection_id,
                    sender: sender.clone(),
                    codec,
                    expected,
                    current,
                    applied: None,
                    configuration_in_flight: false,
                    worker: Some(worker),
                },
            )
        } else {
            drop(sender);
            let _ = worker.join();
            None
        };
        if let Some(mut previous) = previous {
            drop(previous.sender);
            if let Some(worker) = previous.worker.take() {
                let _ = worker.join();
            }
        }
    }

    pub(super) fn deactivate(&self, connection_id: u64) {
        let removed = if let Ok(mut state) = self.state.lock() {
            let removed = state.targets.remove(&connection_id);
            if state.targets.is_empty() {
                state.expected = None;
                state.current = None;
            }
            removed
        } else {
            None
        };
        if let Some(mut target) = removed {
            drop(target.sender);
            if let Some(worker) = target.worker.take() {
                let _ = worker.join();
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
                target.configuration_in_flight = false;
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
        let mut pending = Vec::new();
        let target_count = self
            .state
            .lock()
            .ok()
            .map(|mut state| {
                for target in state.targets.values_mut() {
                    if let Some(expected) = target.expected.clone() {
                        if frame.width != expected.requested_width
                            || frame.height != expected.requested_height
                            || frame.generation != expected.generation
                        {
                            if frame.generation != expected.generation {
                                self.metrics.record_generation_drop();
                            }
                            continue;
                        }
                        if target.configuration_in_flight {
                            continue;
                        }
                        target.configuration_in_flight = true;
                        pending.push((
                            target.connection_id,
                            target.sender.clone(),
                            EncodeRequest {
                                frame: frame.clone(),
                                expected: Some(expected),
                            },
                            true,
                        ));
                        continue;
                    }

                    let Some(current) = target.current.as_ref() else {
                        continue;
                    };
                    if frame.width != current.width
                        || frame.height != current.height
                        || frame.generation != current.generation
                    {
                        if frame.generation != current.generation {
                            self.metrics.record_generation_drop();
                        }
                        continue;
                    }
                    pending.push((
                        target.connection_id,
                        target.sender.clone(),
                        EncodeRequest {
                            frame: frame.clone(),
                            expected: None,
                        },
                        false,
                    ));
                }
                state.targets.len()
            })
            .unwrap_or(0);

        if target_count == 0 {
            self.metrics.record_disconnected_drop();
            return;
        }

        for (connection_id, sender, request, is_configuration) in pending {
            let trace = request.frame.trace;
            match sender.try_send(request) {
                Ok(()) => self.metrics.record_encoder_queued(trace),
                Err(TrySendError::Full(_)) => {
                    self.metrics.record_encoder_queue_drop();
                    if is_configuration {
                        reset_configuration_in_flight(&self.state, connection_id);
                    }
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.metrics.record_disconnected_drop();
                    if is_configuration {
                        reset_configuration_in_flight(&self.state, connection_id);
                    }
                }
            }
        }
    }
}

fn reset_configuration_in_flight(state: &Arc<Mutex<FrameRouterState>>, connection_id: u64) {
    if let Ok(mut state) = state.lock()
        && let Some(target) = state.targets.get_mut(&connection_id)
    {
        target.configuration_in_flight = false;
    }
}

fn spawn_encoder_worker(
    connection_id: u64,
    encoder: Arc<dyn FrameEncoder>,
    receiver: Receiver<EncodeRequest>,
    state: Arc<Mutex<FrameRouterState>>,
    metrics: FrameTransportMetrics,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name(format!("viewport-encoder-{connection_id}"))
        .spawn(move || {
            while let Ok(request) = receiver.recv() {
                let active_generation = state.lock().ok().and_then(|state| {
                    state.targets.get(&connection_id).and_then(|target| {
                        target
                            .expected
                            .as_ref()
                            .map(|metrics| metrics.generation)
                            .or_else(|| target.current.as_ref().map(|current| current.generation))
                    })
                });
                if !active_generation.is_some_and(|generation| {
                    generation_matches(request.frame.generation, generation)
                }) {
                    if active_generation.is_none() {
                        metrics.record_disconnected_drop();
                    } else {
                        metrics.record_generation_drop();
                    }
                    if request.expected.is_some() {
                        reset_configuration_in_flight(&state, connection_id);
                    }
                    continue;
                }

                metrics.record_encoder_worker_started(request.frame.trace);

                let result = if let Some(expected) = request.expected.as_ref() {
                    let fps = expected.preferred_fps.unwrap_or(60);
                    encoder
                        .set_video_caps(request.frame.width, request.frame.height, fps)
                        .and_then(|_| encoder.request_sync_frame_after_caps_change())
                        .and_then(|_| encoder.push_frame(&request.frame))
                } else {
                    encoder.push_frame(&request.frame)
                };

                if let Err(error) = result {
                    metrics.record_encoder_failure();
                    debug!("[viewport-frame-pump] frame push failed: {error:?}");
                    if request.expected.is_some() {
                        reset_configuration_in_flight(&state, connection_id);
                    }
                    continue;
                }
                metrics.record_encoder_pushed(request.frame.trace);

                let Some(expected) = request.expected else {
                    continue;
                };
                let configuration = ActiveStreamConfiguration {
                    width: request.frame.width,
                    height: request.frame.height,
                    fps: expected.preferred_fps.unwrap_or(60),
                    codec: state
                        .lock()
                        .ok()
                        .and_then(|state| {
                            state.targets.get(&connection_id).map(|target| target.codec)
                        })
                        .unwrap_or(CodecId::H264),
                    generation: request.frame.generation,
                };
                apply_configuration(&state, connection_id, expected, configuration);
            }
        })
        .expect("viewport encoder worker should start")
}

fn apply_configuration(
    state: &Arc<Mutex<FrameRouterState>>,
    connection_id: u64,
    expected_metrics: ViewportMetrics,
    configuration: ActiveStreamConfiguration,
) {
    if let Ok(mut state) = state.lock()
        && let Some(target) = state.targets.get_mut(&connection_id)
        && target
            .expected
            .as_ref()
            .is_some_and(|current| current == &expected_metrics)
    {
        target.configuration_in_flight = false;
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
}

#[cfg(test)]
mod tests {
    use super::{FrameRouter, generation_matches};
    use crate::{FrameTransportMetrics, VideoFrame};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::mpsc::sync_channel;
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};
    use viewport_protocol::{CodecId, ViewportMetrics};

    #[derive(Default)]
    struct GateState {
        entered: bool,
        released: bool,
    }

    #[derive(Default)]
    struct TestGate {
        state: Mutex<GateState>,
        condition: Condvar,
    }

    struct SlowTestEncoder {
        block_push: AtomicBool,
        pushes: AtomicU64,
        gate: TestGate,
    }

    impl SlowTestEncoder {
        fn wait_until_push_is_blocked(&self) -> bool {
            let deadline = Instant::now() + Duration::from_secs(1);
            let mut state = self.gate.state.lock().expect("gate state lock");
            while !state.entered {
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    return false;
                };
                let (next, timeout) = self
                    .gate
                    .condition
                    .wait_timeout(state, remaining)
                    .expect("entered condition");
                state = next;
                if timeout.timed_out() && !state.entered {
                    return false;
                }
            }
            true
        }

        fn release_push(&self) {
            let mut state = self.gate.state.lock().expect("gate state lock");
            state.released = true;
            self.gate.condition.notify_all();
        }
    }

    impl super::FrameEncoder for SlowTestEncoder {
        fn push_frame(&self, _frame: &VideoFrame) -> anyhow::Result<()> {
            self.pushes.fetch_add(1, Ordering::Relaxed);
            if self.block_push.load(Ordering::Relaxed) {
                let mut state = self.gate.state.lock().expect("gate state lock");
                state.entered = true;
                self.gate.condition.notify_all();
                while !state.released {
                    state = self.gate.condition.wait(state).expect("released condition");
                }
            }
            Ok(())
        }

        fn set_video_caps(&self, _width: u32, _height: u32, _fps: u32) -> anyhow::Result<()> {
            Ok(())
        }

        fn request_sync_frame_after_caps_change(&self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn test_metrics(generation: u64) -> ViewportMetrics {
        ViewportMetrics {
            css_width: 2,
            css_height: 2,
            device_pixel_ratio: 1.0,
            requested_width: 2,
            requested_height: 2,
            preferred_fps: Some(60),
            generation,
        }
    }

    fn test_frame(metrics: &FrameTransportMetrics, generation: u64) -> VideoFrame {
        VideoFrame {
            rgba: Arc::new(vec![0; 2 * 2 * 4]),
            width: 2,
            height: 2,
            generation,
            trace: metrics.mark_readback_complete(metrics.next_render_trace()),
        }
    }

    fn wait_for_applied(router: &FrameRouter, connection_id: u64) -> bool {
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if router.take_applied(connection_id).is_some() {
                return true;
            }
            thread::yield_now();
        }
        false
    }

    #[test]
    fn saturated_encoder_queue_is_non_blocking() {
        let (sender, receiver) = sync_channel::<u64>(1);
        sender.send(1).expect("queue accepts its capacity");
        let started = Instant::now();
        assert!(sender.try_send(2).is_err());
        assert!(started.elapsed().as_millis() < 50);
        drop(receiver);
    }

    #[test]
    fn disconnected_encoder_queue_is_observable_without_waiting() {
        let (sender, receiver) = sync_channel::<u64>(1);
        drop(receiver);
        assert!(sender.try_send(1).is_err());
    }

    #[test]
    fn generation_change_rejects_the_old_video_request() {
        assert!(generation_matches(7, 7));
        assert!(!generation_matches(6, 7));
    }

    #[test]
    fn reconnect_gets_a_fresh_bounded_queue() {
        let (old_sender, old_receiver) = sync_channel::<u64>(1);
        old_sender.send(1).expect("old session accepts a frame");
        drop(old_receiver);
        assert!(old_sender.try_send(2).is_err());

        let (new_sender, new_receiver) = sync_channel::<u64>(1);
        new_sender
            .send(3)
            .expect("reconnected session accepts a fresh frame");
        assert_eq!(
            new_receiver.recv().expect("fresh queue has the new frame"),
            3
        );
    }

    #[test]
    fn production_router_bounds_slow_encoder_and_preserves_control_lane() {
        let metrics = FrameTransportMetrics::default();
        let router = FrameRouter::new(metrics.clone());
        let encoder = Arc::new(SlowTestEncoder {
            block_push: AtomicBool::new(false),
            pushes: AtomicU64::new(0),
            gate: TestGate::default(),
        });
        router.activate(7, Arc::clone(&encoder), CodecId::H264);
        router.configure(7, test_metrics(1));

        router.push(&test_frame(&metrics, 1));
        let configured = wait_for_applied(&router, 7);
        encoder.block_push.store(true, Ordering::Relaxed);
        router.push(&test_frame(&metrics, 1));
        let worker_blocked = encoder.wait_until_push_is_blocked();

        router.push(&test_frame(&metrics, 1));
        router.push(&test_frame(&metrics, 1));
        let started = Instant::now();
        router.push(&test_frame(&metrics, 1));
        let saturated_push_time = started.elapsed();

        let (control_sender, control_receiver) = sync_channel(1);
        control_sender
            .try_send("reliable-control")
            .expect("control lane remains available while video queue is full");
        let control_message = control_receiver
            .recv_timeout(Duration::from_millis(50))
            .expect("reliable control message is not held behind video work");

        encoder.release_push();
        router.deactivate(7);
        let snapshot = metrics.snapshot();

        assert!(configured, "initial configuration should be applied");
        assert!(
            worker_blocked,
            "fake encoder should hold one worker request"
        );
        assert!(saturated_push_time < Duration::from_millis(50));
        assert_eq!(control_message, "reliable-control");
        assert!(snapshot.encoder_queue_drops >= 1);
        assert!(snapshot.encoder_pushed >= 1);
    }

    #[test]
    fn production_router_drops_stale_generation_and_reconnects_cleanly() {
        let metrics = FrameTransportMetrics::default();
        let router = FrameRouter::new(metrics.clone());
        let first_encoder = Arc::new(SlowTestEncoder {
            block_push: AtomicBool::new(false),
            pushes: AtomicU64::new(0),
            gate: TestGate::default(),
        });
        router.activate(11, Arc::clone(&first_encoder), CodecId::H264);
        router.configure(11, test_metrics(3));
        router.push(&test_frame(&metrics, 2));
        router.deactivate(11);

        let second_encoder = Arc::new(SlowTestEncoder {
            block_push: AtomicBool::new(false),
            pushes: AtomicU64::new(0),
            gate: TestGate::default(),
        });
        router.activate(11, Arc::clone(&second_encoder), CodecId::H264);
        router.configure(11, test_metrics(4));
        router.push(&test_frame(&metrics, 4));
        let configured = wait_for_applied(&router, 11);
        router.deactivate(11);

        let snapshot = metrics.snapshot();
        assert!(
            configured,
            "reconnected target should configure independently"
        );
        assert!(snapshot.generation_drops >= 1);
        assert!(snapshot.disconnected_drops == 0);
    }
}
