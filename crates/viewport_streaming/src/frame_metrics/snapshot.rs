use serde::{Deserialize, Serialize};

/// Serializable snapshot used by benchmark reports and checkpoint artifacts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FrameTransportSnapshot {
    pub last_queued_sequence: u64,
    pub last_encoded_sequence: u64,
    pub readback_completions: u64,
    pub captured_frames: u64,
    pub queued_frames: u64,
    pub queue_full_drops: u64,
    pub invalid_readbacks: u64,
    #[serde(default)]
    pub readback_identity_misses: u64,
    #[serde(default)]
    pub readback_correlation_overflows: u64,
    #[serde(default)]
    pub readback_correlation_high_water: u64,
    pub readback_copy_bytes: u64,
    pub readback_repacked_frames: u64,
    pub generation_drops: u64,
    pub encoder_submitted: u64,
    pub encoder_queue_drops: u64,
    pub encoder_pushed: u64,
    pub encoder_failures: u64,
    pub disconnected_drops: u64,
    pub measurement_elapsed_ms: Option<f64>,
    pub readback_fps: Option<f64>,
    pub encoder_push_fps: Option<f64>,
    pub render_to_readback_avg_ms: Option<f64>,
    pub render_to_readback_max_ms: Option<f64>,
    pub readback_to_queue_avg_ms: Option<f64>,
    pub readback_to_queue_max_ms: Option<f64>,
    pub readback_to_encoder_queue_avg_ms: Option<f64>,
    pub readback_to_encoder_queue_max_ms: Option<f64>,
    pub readback_to_encoder_worker_avg_ms: Option<f64>,
    pub readback_to_encoder_worker_max_ms: Option<f64>,
    pub readback_to_encoder_push_avg_ms: Option<f64>,
    pub readback_to_encoder_push_max_ms: Option<f64>,
}
