use gstreamer::prelude::*;
use viewport_streaming::{CodecCapabilities, EncodePipeline, StreamingConfig, VideoCodec};

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
    let config = StreamingConfig::from_preset(viewport_streaming::StreamingPreset::Adaptive);
    if CodecCapabilities::probe().av1_encoder.is_none() {
        return;
    }

    let pipeline = EncodePipeline::new(&config, VideoCodec::AV1)
        .expect("AV1 encoder is present but the AV1 pipeline could not be created");
    pipeline
        .prepare_video_offer(config.width, config.height)
        .expect("AV1 pipeline did not negotiate RTP caps");
}
