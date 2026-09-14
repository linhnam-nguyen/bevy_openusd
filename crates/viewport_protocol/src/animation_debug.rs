//! Compact, optional animation diagnostics for the reliable application path.
//!
//! The video media path carries animation pixels. This contract is limited to
//! scalar identities and counters so a future debug request cannot accidentally
//! grow into a frame or transform transport.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationDebugMessageKind {
    Snapshot,
    Failure,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimationDebugSnapshot {
    pub stage_session_id: Option<u64>,
    pub stage_generation: Option<u64>,
    pub time_code: Option<f64>,
    pub render_sequence: Option<u64>,
    pub render_hash: Option<u64>,
    pub render_mad_luma: Option<f64>,
    pub decoded_frames: Option<u64>,
    pub presented_frames: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimationDebugMessage {
    pub kind: AnimationDebugMessageKind,
    pub snapshot: AnimationDebugSnapshot,
}

#[cfg(test)]
mod tests {
    use super::{
        AnimationDebugMessage, AnimationDebugMessageKind, AnimationDebugSnapshot,
    };

    const MAX_DEBUG_MESSAGE_BYTES: usize = 1_024;

    #[test]
    fn debug_message_serialization_stays_below_one_kibibyte() {
        let message = AnimationDebugMessage {
            kind: AnimationDebugMessageKind::Snapshot,
            snapshot: AnimationDebugSnapshot {
                stage_session_id: Some(u64::MAX),
                stage_generation: Some(u64::MAX),
                time_code: Some(f64::MAX),
                render_sequence: Some(u64::MAX),
                render_hash: Some(u64::MAX),
                render_mad_luma: Some(f64::MAX),
                decoded_frames: Some(u64::MAX),
                presented_frames: Some(u64::MAX),
            },
        };
        let encoded = serde_json::to_vec(&message).expect("debug message serializes");

        assert!(encoded.len() < MAX_DEBUG_MESSAGE_BYTES);
        assert!(!encoded.windows(4).any(|window| window == b"rgba"));
    }
}
