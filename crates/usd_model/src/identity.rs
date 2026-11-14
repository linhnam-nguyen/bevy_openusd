//! Stable identity values for semantic model objects.

use serde::{Deserialize, Serialize};

/// Identity of one logical model object across revisions.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct EntityKey(pub String);

impl EntityKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for EntityKey {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for EntityKey {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

/// Source used to resolve an [`EntityKey`].
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum IdentitySource {
    RevitUniqueId,
    IfcGuid,
    ApplicationGuid,
    AssetIdentifier,
    PrimPath,
    Synthetic,
}

#[cfg(test)]
#[path = "../tests/unit/identity_tests.rs"]
mod tests;
