use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::router::FrameRouter;
use crate::VideoFrame;
use crate::frame_metrics::FrameTransportMetrics;

/// Owns the blocking Bevy-frame receiver and forwards frames to all admitted
/// sessions. Frames arriving while disconnected are intentionally dropped.
pub(crate) struct FramePump {
    router: FrameRouter,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl FramePump {
    pub(crate) fn new(receiver: Receiver<VideoFrame>, metrics: FrameTransportMetrics) -> Self {
        let router = FrameRouter::new(metrics);
        let worker_router = router.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);

        let worker = thread::Builder::new()
            .name("viewport-frame-pump".to_owned())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(frame) => worker_router.push(&frame),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .expect("viewport frame pump thread should start");

        Self {
            router,
            stop,
            worker: Some(worker),
        }
    }

    pub(crate) fn router(&self) -> FrameRouter {
        self.router.clone()
    }
}

impl Drop for FramePump {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
