//! Test-only helpers attached to the owning production module.
use super::*;

impl PendingBimProperties {
    pub(crate) fn has_request(&self) -> bool {
        self.request_id.is_some()
    }
}
