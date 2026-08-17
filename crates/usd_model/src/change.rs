//! Semantic change classification values.

/// Whether an entity is present in the baseline and/or current snapshot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PresenceState {
    Added,
    Removed,
    Existing,
}

bitflags::bitflags! {
    /// Independent semantic dimensions that changed for an existing entity.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub struct ChangeFlags: u16 {
        const TRANSFORM = 1 << 0;
        const GEOMETRY  = 1 << 1;
        const METADATA  = 1 << 2;
        const PATH      = 1 << 3;
    }
}

#[cfg(test)]
mod tests {
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
}
