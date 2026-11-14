use super::*;

#[test]
fn test_model_usda_fixture_validity() {
    assert!(!MODEL_USDA.is_empty());
    let stage = stage_from_usda(std::str::from_utf8(MODEL_USDA).unwrap())
        .expect("MODEL_USDA fixture opens as valid Stage");
    assert!(
        stage
            .prim(openusd::sdf::path("/World").unwrap())
            .is_valid()
            .unwrap()
    );
    assert!(
        stage
            .prim(openusd::sdf::path("/World/Building/Wall_A").unwrap())
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
            .is_valid()
            .unwrap()
    );
    for group in ["A", "B", "C"] {
        let group_path = format!("/World/{group}");
        assert!(
            stage
                .prim(openusd::sdf::path(&group_path).unwrap())
                .is_valid()
                .unwrap()
        );
        for i in 0..10 {
            let prim_path = format!("/World/{group}/{group}{i}");
            assert!(
                stage
                    .prim(openusd::sdf::path(&prim_path).unwrap())
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
            .is_valid()
            .unwrap()
    );
    assert!(
        stage
            .prim(openusd::sdf::path("/World/B").unwrap())
            .is_valid()
            .unwrap()
    );
    assert!(
        stage
            .prim(openusd::sdf::path("/World/C").unwrap())
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
        .join("stages/empty.usda");
    assert_eq!(resolved, expected);
    assert!(resolved.is_file());
    assert!(std::panic::catch_unwind(|| resolve_fixture_path("../Cargo.toml")).is_err());
    assert!(
        std::panic::catch_unwind(|| resolve_fixture_path("__missing_bo_m6_fixture__.usda"))
            .is_err()
    );
}
