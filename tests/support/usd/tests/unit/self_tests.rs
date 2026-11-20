use super::*;

#[test]
fn test_model_usda_fixture_validity() {
    assert!(!MODEL_USDA.is_empty());
    let stage = stage_from_usda(std::str::from_utf8(MODEL_USDA).unwrap())
        .expect("MODEL_USDA fixture opens as valid Stage");
    assert!(
        stage
            .prim(openusd::sdf::path("/World").unwrap())
            .unwrap()
            .is_valid()
            .unwrap()
    );
    assert!(
        stage
            .prim(openusd::sdf::path("/World/Building/Wall_A").unwrap())
            .unwrap()
            .is_valid()
            .unwrap()
    );
}

#[test]
fn test_synthetic_wide_stage_structure() {
    let stage = make_synthetic_wide_stage();
    assert!(
        stage
            .prim(openusd::sdf::path("/World").unwrap())
            .unwrap()
            .is_valid()
            .unwrap()
    );
    for group in ["A", "B", "C"] {
        let group_path = format!("/World/{group}");
        assert!(
            stage
                .prim(openusd::sdf::path(&group_path).unwrap())
                .unwrap()
                .is_valid()
                .unwrap()
        );
        for i in 0..10 {
            let prim_path = format!("/World/{group}/{group}{i}");
            assert!(
                stage
                    .prim(openusd::sdf::path(&prim_path).unwrap())
                    .unwrap()
                    .is_valid()
                    .unwrap()
            );
        }
    }
}

#[test]
fn test_deep_overlap_stage_structure() {
    let stage = make_deep_overlap_stage();
    assert!(
        stage
            .prim(openusd::sdf::path("/World/A/Child/Leaf").unwrap())
            .unwrap()
            .is_valid()
            .unwrap()
    );
    assert!(
        stage
            .prim(openusd::sdf::path("/World/B").unwrap())
            .unwrap()
            .is_valid()
            .unwrap()
    );
    assert!(
        stage
            .prim(openusd::sdf::path("/World/C").unwrap())
            .unwrap()
            .is_valid()
            .unwrap()
    );
}

#[test]
fn test_resolve_fixture_path() {
    let resolved = resolve_fixture_path("empty.usda");
    let expected = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("fixtures/usd/empty.usda");
    assert_eq!(resolved, expected);
    assert_eq!(crate::usd_fixtures_dir(), expected.parent().unwrap());
    assert_eq!(
        crate::resolve_fixture_path_unchecked("missing-unchecked.usda"),
        crate::usd_fixtures_dir().join("missing-unchecked.usda")
    );
    assert!(resolved.is_file());
    assert!(std::panic::catch_unwind(|| resolve_fixture_path("../Cargo.toml")).is_err());
    assert!(std::panic::catch_unwind(|| resolve_fixture_path("")).is_err());
    assert!(
        std::panic::catch_unwind(|| resolve_fixture_path("__missing_bo_m6_fixture__.usda"))
            .is_err()
    );
}

#[test]
fn test_resolve_asset_and_external_paths() {
    let asset = crate::resolve_asset_path("two_xforms.usda");
    assert!(asset.is_file());
    assert!(asset.ends_with(Path::new("assets/two_xforms.usda")));
    assert!(std::panic::catch_unwind(|| crate::resolve_asset_path("../Cargo.toml")).is_err());
    assert!(
        std::panic::catch_unwind(|| crate::resolve_external_asset_path("../secret.usdc")).is_err()
    );
    let external = crate::resolve_external_asset_path("Omniverse/V2/Projet1.usdc");
    assert!(external.ends_with(Path::new("external_assets/Omniverse/V2/Projet1.usdc")));
}

#[test]
fn test_resolve_crate_fixture_path() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .unwrap()
        .join("crates")
        .join("usd_semantic");
    let local_fixture =
        crate::resolve_crate_fixture_path(manifest_dir.to_str().unwrap(), "identity_moved.usda");
    assert!(local_fixture.is_file());
    assert!(
        std::panic::catch_unwind(|| crate::resolve_crate_fixture_path(
            manifest_dir.to_str().unwrap(),
            "../Cargo.toml"
        ))
        .is_err()
    );
}
