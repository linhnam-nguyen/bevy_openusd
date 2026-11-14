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
#[path = "../tests/unit/change_tests.rs"]
mod tests;
