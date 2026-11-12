use super::*;

#[test]
fn managed_ignore_is_idempotent_and_preserves_unrelated_bytes() {
    let existing = b"target/\n# local note\n";
    let first = merge_managed_ignore(existing).unwrap();
    let second = merge_managed_ignore(&first).unwrap();

    assert_eq!(first, second);
    assert!(first.starts_with(existing));
    assert_eq!(first.iter().filter(|byte| **byte == b'\n').count(), 6);
}

#[test]
fn managed_ignore_replaces_only_its_own_block() {
    let existing = b"keep-before\n# BEGIN USDHub managed local state\nold\n# END USDHub managed local state\nkeep-after\n";
    let merged = merge_managed_ignore(existing).unwrap();
    assert_eq!(
        String::from_utf8(merged).unwrap(),
        "keep-before\n# BEGIN USDHub managed local state\n.usdhub/cache/\n.usdhub/recovery/\n# END USDHub managed local state\nkeep-after\n"
    );
}

#[test]
fn broad_usdhub_rule_is_a_conflict() {
    for rule in [
        ".usdhub/",
        "/.usdhub/",
        "**/.usdhub",
        ".usdhub/*",
        ".usdhub/**",
        "/.usdhub/*",
        "**/.usdhub/**",
    ] {
        assert!(has_broad_usdhub_ignore(rule.as_bytes()).unwrap(), "{rule}");
    }
    assert!(!has_broad_usdhub_ignore(b".usdhub/cache/\n").unwrap());
    assert!(!has_broad_usdhub_ignore(b".usdhub/recovery/**\n").unwrap());
}
