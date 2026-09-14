use super::{client_content_failure, client_presentation_failure};

fn client_proof(
    proof: &str,
    presented_delta: Option<u64>,
    delivery_delta: Option<u64>,
) -> serde_json::Value {
    serde_json::json!({
        "presented_delta": presented_delta,
        "delivery_delta": delivery_delta,
        "proofs": [proof],
    })
}

#[test]
fn compositor_proof_requires_presented_progress() {
    assert_eq!(
        client_presentation_failure(&client_proof("compositor", Some(1), None)),
        None
    );
    assert_eq!(
        client_presentation_failure(&client_proof("compositor", Some(0), None)),
        Some("CLIENT_PRESENTATION_STALLED")
    );
}

#[test]
fn labelled_delivery_fallback_does_not_require_compositor_progress() {
    assert_eq!(
        client_presentation_failure(&client_proof("rtp_decoded", None, Some(1))),
        None
    );
    assert_eq!(
        client_presentation_failure(&client_proof("rtp_decoded", None, Some(0))),
        Some("CLIENT_PRESENTATION_STALLED")
    );
}

#[test]
fn missing_presentation_proof_is_stalled() {
    assert_eq!(
        client_presentation_failure(&serde_json::json!({
            "presented_delta": None::<u64>,
            "delivery_delta": None::<u64>,
            "proofs": [],
        })),
        Some("CLIENT_PRESENTATION_STALLED")
    );
    assert_eq!(
        client_presentation_failure(&serde_json::json!({
            "presented_delta": None::<u64>,
            "delivery_delta": None::<u64>,
        })),
        Some("CLIENT_PRESENTATION_STALLED")
    );
}

#[test]
fn unsupported_client_content_probe_is_informational() {
    assert_eq!(
        client_content_failure(&serde_json::json!({
            "content_signature_supported": false,
            "content_hashes": []
        })),
        None
    );
    assert_eq!(
        client_content_failure(&serde_json::json!({
            "content_hashes": []
        })),
        None
    );
}

#[test]
fn supported_static_client_content_is_classified() {
    assert_eq!(
        client_content_failure(&serde_json::json!({
            "content_signature_supported": true,
            "content_hashes": ["deadbeef", "deadbeef"]
        })),
        Some("CLIENT_CONTENT_STATIC")
    );
}
