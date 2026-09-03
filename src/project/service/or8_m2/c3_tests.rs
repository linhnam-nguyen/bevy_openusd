use super::*;

#[test]
fn c3_smoke_exercises_seeded_import_link_composition_for_four_seeds() {
    for seed in rng::M2_SEEDS.iter().take(4).copied() {
        composition::run_seed(seed).unwrap_or_else(|error| {
            panic!("C3 seed {seed:#018X} failed: {error}");
        });
    }
}
