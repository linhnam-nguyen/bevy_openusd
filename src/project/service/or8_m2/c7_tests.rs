use super::*;

#[test]
fn c7_smoke_scopes_cloned_project_registration_for_four_seeds() {
    for seed in rng::M2_SEEDS.iter().take(4).copied() {
        registration::run_seed(seed).unwrap_or_else(|error| {
            panic!("C7 seed {seed:#018X} failed: {error}");
        });
    }
}
