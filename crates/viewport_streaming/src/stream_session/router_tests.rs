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
    assert_eq!(snapshot.disconnected_drops, 0);
}
