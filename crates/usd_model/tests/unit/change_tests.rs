use super::*;

#[test]
fn presence_states_are_exhaustive_values() {
    assert_eq!(PresenceState::Added, PresenceState::Added);
    assert_ne!(PresenceState::Added, PresenceState::Removed);
    assert_ne!(PresenceState::Removed, PresenceState::Existing);
}

#[test]
fn change_flags_combine_independent_dimensions() {
    let flags = ChangeFlags::TRANSFORM | ChangeFlags::METADATA;

    assert!(flags.contains(ChangeFlags::TRANSFORM));
    assert!(flags.contains(ChangeFlags::METADATA));
    assert!(!flags.contains(ChangeFlags::GEOMETRY));
    assert_eq!(flags.bits(), (1 << 0) | (1 << 2));
}
