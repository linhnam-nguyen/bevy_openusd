use super::caps::{raw_video_caps, rgba_byte_count, sync_frame_event};

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

#[test]
fn raw_caps_preserve_the_end_to_end_configuration_matrix() {
    gstreamer::init().expect("GStreamer initializes for raw caps verification");
    for (width, height) in [(1280, 720), (1920, 1080), (2560, 1440)] {
        for fps in [30, 60, 120] {
            let caps = raw_video_caps(width, height, fps).expect("matrix dimensions are valid");
            let structure = caps.structure(0).expect("raw caps contain one structure");
            assert_eq!(structure.get::<i32>("width").unwrap(), width as i32);
            assert_eq!(structure.get::<i32>("height").unwrap(), height as i32);
            assert_eq!(
                structure.get::<gstreamer::Fraction>("framerate").unwrap(),
                gstreamer::Fraction::new(fps as i32, 1)
            );
        }
    }
}
