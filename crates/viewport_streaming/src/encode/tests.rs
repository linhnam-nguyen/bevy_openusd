use gstreamer::prelude::*;

use super::caps::{CodecCapabilities, VideoCodec, rgba_byte_count, sync_frame_event};
use super::pipeline::EncodePipeline;
use crate::config::StreamingConfig;

#[test]
fn test_pipeline_creation() {
    let config = StreamingConfig::default();
    let pipeline = EncodePipeline::new(&config, VideoCodec::H264);
    let pipeline = pipeline.expect("Failed to create H.264 encode pipeline");
    assert_eq!(
        pipeline
            .webrtc()
            .property::<gstreamer_webrtc::WebRTCBundlePolicy>("bundle-policy"),
        gstreamer_webrtc::WebRTCBundlePolicy::MaxBundle
    );
}

#[test]
fn av1_pipeline_creation_skips_h26x_config_interval() {
    let config = StreamingConfig::default();
    if CodecCapabilities::probe().av1_encoder.is_none() {
        return;
    }

    let pipeline = EncodePipeline::new(&config, VideoCodec::AV1)
        .expect("AV1 encoder is present but the AV1 pipeline could not be created");
    assert_eq!(pipeline.selected_codec(), VideoCodec::AV1);
}

#[test]
fn av1_pipeline_can_negotiate_rtp_caps() {
    let config = StreamingConfig::from_preset(crate::config::StreamingPreset::Adaptive);
    if CodecCapabilities::probe().av1_encoder.is_none() {
        return;
    }

    let pipeline = EncodePipeline::new(&config, VideoCodec::AV1)
        .expect("AV1 encoder is present but the AV1 pipeline could not be created");
    pipeline
        .prepare_video_offer(config.width, config.height)
        .expect("AV1 pipeline did not negotiate RTP caps");
}

#[test]
fn rgba_byte_count_matches_active_caps_shape() {
    assert_eq!(rgba_byte_count(1280, 720).unwrap(), 3_686_400);
    assert!(rgba_byte_count(u32::MAX, u32::MAX).is_err());
}

#[test]
fn sync_frame_request_is_codec_neutral_and_includes_supported_headers() {
    gstreamer::init().expect("GStreamer initializes for video-event construction");
    let event = sync_frame_event();
    let request = gstreamer_video::UpstreamForceKeyUnitEvent::parse(event.as_ref())
        .expect("the resize request is an upstream force-key-unit event");

    assert!(request.all_headers);
}
