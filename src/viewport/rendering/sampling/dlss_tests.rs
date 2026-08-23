use super::DlssCapability;

#[test]
fn runtime_support_requires_an_opted_in_build() {
    assert!(!DlssCapability::from_probe(false, true).supported());
    assert!(!DlssCapability::from_probe(true, false).supported());
    assert!(DlssCapability::from_probe(true, true).supported());
}

#[test]
fn default_capability_is_fail_closed() {
    let capability = DlssCapability::default();

    assert!(!capability.runtime_supported);
    assert_eq!(
        capability.supported(),
        capability.compiled && capability.runtime_supported
    );
}
