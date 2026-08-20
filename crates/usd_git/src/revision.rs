use std::fmt;

/// A fully resolved Git object ID.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RevisionId(String);

impl RevisionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for RevisionId {
    fn from(id: String) -> Self {
        Self::new(id)
    }
}

impl From<&str> for RevisionId {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

impl fmt::Display for RevisionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A revision expression accepted by Git's revision resolver.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RevisionSpec {
    Head,
    Name(String),
    Id(RevisionId),
}

impl RevisionSpec {
    pub fn head() -> Self {
        Self::Head
    }

    pub fn name(name: impl Into<String>) -> Self {
        Self::Name(name.into())
    }

    pub fn id(id: impl Into<RevisionId>) -> Self {
        Self::Id(id.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Head => "HEAD",
            Self::Name(name) => name,
            Self::Id(id) => id.as_str(),
        }
    }
}

impl From<&str> for RevisionSpec {
    fn from(spec: &str) -> Self {
        Self::Name(spec.to_owned())
    }
}

impl From<String> for RevisionSpec {
    fn from(spec: String) -> Self {
        Self::Name(spec)
    }
}

/// A resolved commit revision.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Revision {
    id: RevisionId,
}

impl Revision {
    pub(crate) fn from_id(id: RevisionId) -> Self {
        Self { id }
    }

    pub fn id(&self) -> &RevisionId {
        &self.id
    }
}
