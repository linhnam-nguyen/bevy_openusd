use super::*;

#[test]
fn startup_publishes_initial_requested_and_effective_target() {
    let cadence = RendererCadence::new(Some(60));

    assert_eq!(cadence.requested_fps(), Some(60));
    assert_eq!(cadence.effective_renderer_target_fps(), Some(60));
    assert_eq!(cadence.applied_generation(), 0);
    assert_eq!(
        cadence.wait_duration(),
        Some(Duration::from_secs_f64(1.0 / 60.0))
    );
}

#[test]
fn cadence_transitions_30_to_60_to_120_only_after_application() {
    let mut cadence = RendererCadence::new(Some(60));

    assert!(cadence.request_local(Some(30), "fps-30".to_owned()));
    assert_eq!(cadence.effective_renderer_target_fps(), Some(60));
    assert_eq!(cadence.apply_pending().unwrap().fps, Some(30));
    assert_eq!(cadence.effective_renderer_target_fps(), Some(30));

    assert!(cadence.request_local(Some(60), "fps-60".to_owned()));
    assert_eq!(cadence.effective_renderer_target_fps(), Some(30));
    assert_eq!(cadence.apply_pending().unwrap().fps, Some(60));

    assert!(cadence.request_local(Some(120), "fps-120".to_owned()));
    assert_eq!(cadence.effective_renderer_target_fps(), Some(60));
    assert_eq!(cadence.apply_pending().unwrap().fps, Some(120));
    assert_eq!(
        cadence.wait_duration(),
        Some(Duration::from_secs_f64(1.0 / 120.0))
    );
}

#[test]
fn stale_stream_generation_cannot_replace_a_newer_request() {
    let mut cadence = RendererCadence::new(Some(60));

    assert!(cadence.request_stream(Some(30), 2));
    assert!(!cadence.request_stream(Some(120), 1));
    assert_eq!(cadence.requested_fps(), Some(30));
    assert_eq!(cadence.apply_pending().unwrap().fps, Some(30));
}

#[test]
fn accepted_stream_fps_survives_a_same_frame_local_request() {
    let mut cadence = RendererCadence::new(Some(60));

    assert!(cadence.request_stream(Some(120), 2));
    assert!(!cadence.request_local(Some(60), "presentation-1".to_owned()));

    let applied = cadence
        .apply_pending()
        .expect("stream cadence must remain queued");
    assert_eq!(applied.fps, Some(120));
    assert_eq!(applied.request_id, None);
    assert_eq!(cadence.effective_renderer_target_fps(), Some(120));
}

#[test]
fn local_fps_has_explicit_priority_but_does_not_discard_stream_fps() {
    let mut cadence = RendererCadence::new(Some(60));

    assert!(cadence.request_stream(Some(120), 2));
    assert!(cadence.request_local(Some(30), "presentation-1".to_owned()));

    assert_eq!(cadence.apply_pending().unwrap().fps, Some(30));
    assert_eq!(cadence.apply_pending().unwrap().fps, Some(120));
}

#[test]
fn fps_only_stream_update_does_not_own_dimensions() {
    let mut cadence = RendererCadence::new(Some(30));
    assert!(cadence.request_stream(Some(120), 3));
    let applied = cadence.apply_pending().unwrap();

    assert_eq!(applied.fps, Some(120));
    assert_eq!(cadence.effective_renderer_target_fps(), Some(120));
    // Dimensions are intentionally absent from this resource: stream
    // resize ownership remains with OffscreenTarget.
}

#[test]
fn effective_state_is_not_published_until_pending_request_is_applied() {
    let mut cadence = RendererCadence::new(Some(60));
    assert!(cadence.request_local(None, "uncapped".to_owned()));

    assert_eq!(cadence.effective_renderer_target_fps(), Some(60));
    let applied = cadence.apply_pending().unwrap();
    assert!(applied.changed);
    assert_eq!(applied.request_id.as_deref(), Some("uncapped"));
    assert_eq!(cadence.effective_renderer_target_fps(), None);
}
