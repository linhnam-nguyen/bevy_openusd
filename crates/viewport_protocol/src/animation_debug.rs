//! Compact, optional animation diagnostics for the reliable application path.
//!
//! Render pixels and luma arrays stay on their owning sides. This contract
//! carries only scalar evidence for the five named samples.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationDebugSampleId {
    T0,
    T1,
    T0RoundTrip,
    StaticT0,
    StaticT1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationPresentationProof {
    Compositor,
    PlaybackQuality,
    RtpDecoded,
    RtpReceived,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimationDebugSnapshot {
    pub sample: Option<AnimationDebugSampleId>,
    pub stage_session_id: Option<u64>,
    pub stage_generation: Option<u64>,
    pub stage_ready: Option<bool>,
    pub animated_prim_count: Option<u32>,
    pub time_code: Option<f64>,
    pub transform_hash: Option<u64>,
    pub render_sequence: Option<u64>,
    pub render_hash: Option<u64>,
    pub render_mean_luma: Option<f64>,
    pub frames_received: Option<u64>,
    pub frames_decoded: Option<u64>,
    pub playback_total_frames: Option<u64>,
    pub delivery_frames: Option<u64>,
    pub presented_frames: Option<u64>,
    pub presentation_proof: Option<AnimationPresentationProof>,
    pub client_content_signature_supported: Option<bool>,
    pub client_content_hash: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::{AnimationDebugSampleId, AnimationDebugSnapshot, AnimationPresentationProof};
    use crate::{
        ViewportWireMessage, encode_json_line,
        viewport::{ViewportEvent, ViewportEventEnvelope},
    };

    const MAX_DEBUG_MESSAGE_BYTES: usize = 1_024;

    fn complete_snapshot() -> AnimationDebugSnapshot {
        AnimationDebugSnapshot {
            sample: Some(AnimationDebugSampleId::T0RoundTrip),
            stage_session_id: Some(u64::MAX),
            stage_generation: Some(u64::MAX),
            stage_ready: Some(true),
            animated_prim_count: Some(u32::MAX),
            time_code: Some(f64::MAX),
            transform_hash: Some(u64::MAX),
            render_sequence: Some(u64::MAX),
            render_hash: Some(u64::MAX),
            render_mean_luma: Some(f64::MAX),
            frames_received: Some(u64::MAX),
            frames_decoded: Some(u64::MAX),
            playback_total_frames: Some(u64::MAX),
            delivery_frames: Some(u64::MAX),
            presented_frames: Some(u64::MAX),
            presentation_proof: Some(AnimationPresentationProof::Compositor),
            client_content_signature_supported: Some(true),
            client_content_hash: Some(u64::MAX),
        }
    }

    #[test]
    fn actual_debug_event_stays_below_one_kibibyte() {
        let message = ViewportWireMessage::Event(ViewportEventEnvelope::new(
            None,
            ViewportEvent::AnimationDebugServerSample {
                snapshot: complete_snapshot(),
            },
        ));
        let encoded = encode_json_line(&message).expect("debug event serializes");

        assert!(encoded.len() < MAX_DEBUG_MESSAGE_BYTES);
        assert!(!encoded.contains("\"rgba\""));
        assert!(!encoded.contains("\"pixels\""));
        assert!(!encoded.contains("\"transforms\""));
    }
}
