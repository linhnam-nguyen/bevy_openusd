use super::*;

#[test]
fn c1_inventory_freezes_the_three_canonical_fixture_records() {
    let (bevy_assets, external_assets) = assets::default_roots();
    let dictionary = assets::inventory(&bevy_assets, &external_assets).expect("inventory assets");
    assert!(!dictionary.assets.is_empty());
    let fixtures = assets::resolve_fixtures(&dictionary, &bevy_assets, &external_assets)
        .expect("resolve canonical fixtures");
    assert_eq!(fixtures.instance_heavy.fixture_eligibility, vec!["A"]);
    assert!(
        fixtures
            .dependency_animation
            .fixture_eligibility
            .iter()
            .any(|value| value == "B")
    );
    assert!(
        fixtures
            .bim_revit
            .fixture_eligibility
            .iter()
            .any(|value| value == "C")
    );
    assert!(
        !fixtures
            .bim_revit
            .fixture_eligibility
            .iter()
            .any(|value| value == "A")
    );
    artifacts::write_dictionary(&artifacts::m2_testspaces_root(), &dictionary)
        .expect("write deterministic M2 asset dictionary");
}

#[test]
fn c1_frozen_fixture_paths_are_not_derived_from_display_names() {
    let (bevy_assets, external_assets) = assets::default_roots();
    let dictionary = assets::inventory(&bevy_assets, &external_assets).expect("inventory assets");
    let fixtures = assets::resolve_fixtures(&dictionary, &bevy_assets, &external_assets)
        .expect("resolve canonical fixtures");
    assert!(
        fixtures
            .instance_path
            .ends_with("external/PointInstancedMedCity.usdz")
    );
    assert!(
        fixtures
            .dependency_animation_path
            .ends_with("external/HumanFemale.usdz")
    );
    assert!(
        fixtures
            .bim_revit_path
            .ends_with("Omniverse/V1/Projet1.usdc")
    );
}

#[test]
fn c1_smoke_uses_only_the_first_four_frozen_seeds() {
    assert_eq!(
        &rng::M2_SEEDS[..4],
        &[
            0x4F52380000000001,
            0x4F52380000000002,
            0x4F52380000000003,
            0x4F52380000000004,
        ]
    );
    let mut stream = rng::DeterministicRng::seeded(rng::M2_SEEDS[3]);
    assert!(stream.choose_index(3) < 3);
}
