use super::*;

#[test]
fn entity_key_is_value_based() {
    let left = EntityKey::new("asset-42");
    let right = EntityKey::from("asset-42");

    assert_eq!(left, right);
    assert_eq!(left.as_str(), "asset-42");
}

#[test]
fn identity_sources_are_distinct_values() {
    assert_ne!(IdentitySource::PrimPath, IdentitySource::Synthetic);
}
