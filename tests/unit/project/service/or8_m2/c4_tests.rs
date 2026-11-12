use super::*;

#[test]
fn c4_smoke_verifies_three_scoped_scene_activations_for_four_seeds() {
    for seed in rng::M2_SEEDS.iter().take(4).copied() {
        activation::run_seed(seed).unwrap_or_else(|error| {
            panic!("C4 seed {seed:#018X} failed: {error}");
        });
    }
}
