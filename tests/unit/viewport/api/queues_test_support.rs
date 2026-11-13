//! Test-only helpers attached to the owning production module.
use super::*;

impl ViewportTreeCommandInbox {
    pub(crate) fn pending_focus_count(&self) -> usize {
        usize::from(self.pending_focus.is_some())
    }
}
