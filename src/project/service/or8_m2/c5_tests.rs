use super::*;

#[test]
fn c5_smoke_proves_scene_label_reuse_and_identity_for_four_seeds() {
    for seed in rng::M2_SEEDS.iter().take(4).copied() {
        lifecycle::run_seed(seed).unwrap_or_else(|error| {
            panic!("C5 seed {seed:#018X} failed: {error}");
        });
    }
}
