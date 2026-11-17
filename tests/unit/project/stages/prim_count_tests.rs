use super::PrimCountFreshness;

#[test]
fn freshness_token_includes_scene_cache_and_runtime_identity() {
    let base = PrimCountFreshness {
        scene_id: None,
        cache_generation: None,
        path: "/tmp/scene.usda".into(),
        activation_generation: 4,
        session_id: 9,
    };
    let mut changed = base.clone();
    changed.activation_generation += 1;
    assert_ne!(base, changed);
    changed = base.clone();
    changed.path = "/tmp/other.usda".into();
    assert_ne!(base, changed);
}
