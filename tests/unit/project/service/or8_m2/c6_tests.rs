use super::*;

#[test]
fn c6_smoke_round_trips_exported_scenes_for_four_seeds() {
    for seed in rng::M2_SEEDS.iter().take(4).copied() {
        export::run_seed(seed).unwrap_or_else(|error| {
            panic!("C6 seed {seed:#018X} failed: {error}");
        });
    }
}
