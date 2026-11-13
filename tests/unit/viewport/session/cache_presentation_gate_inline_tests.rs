use super::*;

#[test]
fn rendered_frame_opens_immediately_but_missing_renderer_uses_bounded_fallback() {
    let scene = SceneId::new_v4();
    let mut rendered = CachePresentationGate::waiting(scene, 9);
    rendered.observe_rendered_frame(scene, 9);
    assert!(rendered.can_open_stage());
    assert!(!rendered.fallback_used);

    let mut fallback = CachePresentationGate::waiting(scene, 9);
    assert!(!fallback.can_open_stage());
    assert!(!fallback.can_open_stage());
    assert!(fallback.can_open_stage());
    assert!(fallback.fallback_used);
}

#[test]
fn a_stale_rendered_generation_does_not_release_the_gate() {
    let scene = SceneId::new_v4();
    let mut gate = CachePresentationGate::waiting(scene, 4);
    gate.observe_rendered_frame(scene, 3);
    assert!(!gate.rendered());
}
