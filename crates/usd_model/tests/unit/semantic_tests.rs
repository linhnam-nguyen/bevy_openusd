use super::*;

#[test]
fn bim_eligibility_requires_normalized_identity_evidence() {
    let generic = SemanticInfo {
        category: Some("Camera".to_owned()),
        type_name: Some("Camera".to_owned()),
        display_name: Some("Helper camera".to_owned()),
        ..Default::default()
    };
    assert!(!generic.is_bim_entity());

    let mut bim = generic;
    bim.bim.family_name = Some("Window family".to_owned());
    assert!(bim.is_bim_entity());
}

#[test]
fn whitespace_only_normalized_identity_is_not_eligibility_evidence() {
    let semantic = SemanticInfo {
        bim: BimIdentity {
            element_id: Some("  ".to_owned()),
            family_name: Some("\t".to_owned()),
        },
        ..Default::default()
    };
    assert!(!semantic.is_bim_entity());
}
