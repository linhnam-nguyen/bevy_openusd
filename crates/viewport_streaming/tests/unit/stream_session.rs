use super::session::{EXPECTED_MEDIA_SECTIONS, normalize_remote_answer};

const ANSWER_WITH_MEDIA_FINGERPRINTS: &str = "v=0\r\n\
o=- 1 1 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
a=group:BUNDLE 0 1\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
c=IN IP4 0.0.0.0\r\n\
a=mid:0\r\n\
a=ice-ufrag:ufrag\r\n\
a=ice-pwd:password\r\n\
a=fingerprint:sha-256 00\r\n\
a=setup:active\r\n\
a=recvonly\r\n\
m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n\
c=IN IP4 0.0.0.0\r\n\
a=mid:1\r\n\
a=ice-ufrag:ufrag\r\n\
a=ice-pwd:password\r\n\
a=fingerprint:sha-256 00\r\n\
a=setup:active\r\n\
a=sctp-port:5000\r\n";

const ANSWER_WITH_SESSION_FINGERPRINT: &str = "v=0\r\n\
o=- 1 1 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
a=fingerprint:sha-256 00\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
c=IN IP4 0.0.0.0\r\n\
a=mid:0\r\n\
a=ice-ufrag:ufrag\r\n\
a=ice-pwd:password\r\n\
a=setup:active\r\n\
a=recvonly\r\n\
m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n\
c=IN IP4 0.0.0.0\r\n\
a=mid:1\r\n\
a=ice-ufrag:ufrag\r\n\
a=ice-pwd:password\r\n\
a=setup:active\r\n\
a=sctp-port:5000\r\n";

#[test]
fn remote_answer_with_media_fingerprints_is_accepted() {
    gstreamer::init().unwrap();
    let sdp =
        gstreamer_sdp::SDPMessage::parse_buffer(ANSWER_WITH_MEDIA_FINGERPRINTS.as_bytes()).unwrap();

    let normalized = normalize_remote_answer(sdp, EXPECTED_MEDIA_SECTIONS).unwrap();
    assert_eq!(normalized.medias_len(), EXPECTED_MEDIA_SECTIONS);
}

#[test]
fn session_fingerprint_is_copied_to_each_media_section() {
    gstreamer::init().unwrap();
    let sdp = gstreamer_sdp::SDPMessage::parse_buffer(ANSWER_WITH_SESSION_FINGERPRINT.as_bytes())
        .unwrap();

    let normalized = normalize_remote_answer(sdp, EXPECTED_MEDIA_SECTIONS).unwrap();
    for index in 0..EXPECTED_MEDIA_SECTIONS {
        assert_eq!(
            normalized
                .media(index)
                .unwrap()
                .attribute_val("fingerprint"),
            Some("sha-256 00")
        );
    }
}

#[test]
fn remote_answer_with_wrong_media_count_is_rejected_before_webrtcbin() {
    gstreamer::init().unwrap();
    let sdp =
        gstreamer_sdp::SDPMessage::parse_buffer(ANSWER_WITH_MEDIA_FINGERPRINTS.as_bytes()).unwrap();

    let error = normalize_remote_answer(sdp, 1).unwrap_err();
    assert!(error.to_string().contains("expected 1"));
}
