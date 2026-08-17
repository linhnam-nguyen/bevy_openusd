//! Stable identity values for semantic model objects.

/// Identity of one logical model object across revisions.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
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
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum IdentitySource {
    RevitUniqueId,
    IfcGuid,
    ApplicationGuid,
    AssetIdentifier,
    PrimPath,
    Synthetic,
}

#[cfg(test)]
mod tests {
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
}
