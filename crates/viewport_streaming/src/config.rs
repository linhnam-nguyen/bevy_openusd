//! Streaming configuration and quality presets for headless Bevy WebRTC.

use std::net::SocketAddr;

/// Preset profiles for streaming quality and frame rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamingPreset {
    /// 1080p @ 120 Hz (18 Mbps H.265 / 14 Mbps AV1 / 28 Mbps H.264)
    #[default]
    Performance,
    /// 2K (2560x1440) @ 60 Hz (16 Mbps H.265)
    Quality,
    /// 720p @ 60 Hz (5 Mbps H.265)
    Adaptive,
}

/// Authoritative streaming runtime parameters.
#[derive(Debug, Clone)]
pub struct StreamingConfig {
    pub preset: StreamingPreset,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub h265_bitrate_kbps: u32,
    pub av1_bitrate_kbps: u32,
    pub h264_bitrate_kbps: u32,
    pub signaling_addr: SocketAddr,
    pub auth_token_secret: Option<String>,
    pub stun_server: String,
    pub turn_server: Option<String>,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self::from_preset(StreamingPreset::Performance)
    }
}

impl StreamingConfig {
    /// Creates a streaming configuration from a preset.
    pub fn from_preset(preset: StreamingPreset) -> Self {
        let signaling_addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let stun_server = String::new();

        match preset {
            StreamingPreset::Performance => Self {
                preset,
                width: 1920,
                height: 1080,
                fps: 120,
                h265_bitrate_kbps: 18000,
                av1_bitrate_kbps: 14000,
                h264_bitrate_kbps: 28000,
                signaling_addr,
                auth_token_secret: None,
                stun_server,
                turn_server: None,
            },
            StreamingPreset::Quality => Self {
                preset,
                width: 2560,
                height: 1440,
                fps: 60,
                h265_bitrate_kbps: 16000,
                av1_bitrate_kbps: 12000,
                h264_bitrate_kbps: 24000,
                signaling_addr,
                auth_token_secret: None,
                stun_server,
                turn_server: None,
            },
            StreamingPreset::Adaptive => Self {
                preset,
                width: 1280,
                height: 720,
                fps: 60,
                h265_bitrate_kbps: 5000,
                av1_bitrate_kbps: 4000,
                h264_bitrate_kbps: 8000,
                signaling_addr,
                auth_token_secret: None,
                stun_server,
                turn_server: None,
            },
        }
    }
}
