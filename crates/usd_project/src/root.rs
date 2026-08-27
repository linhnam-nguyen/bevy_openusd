use serde::{Deserialize, Serialize};

use crate::{ModelId, ProjectDomainError, SceneId};

/// The typed object that owns the Project composition root.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum ProjectRoot {
    Empty,
    Model(ModelId),
    Scene(SceneId),
}

/// Identifies how a model source is resolved without exposing adapter details.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum ModelSourceKind {
    Usd,
    External(String),
}

impl ModelSourceKind {
    pub fn external(value: impl Into<String>) -> Result<Self, ProjectDomainError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ProjectDomainError::EmptyExternalSourceKind);
        }
        Ok(Self::External(value))
    }

    pub fn validate(&self) -> Result<(), ProjectDomainError> {
        if matches!(self, Self::External(value) if value.trim().is_empty()) {
            return Err(ProjectDomainError::EmptyExternalSourceKind);
        }
        Ok(())
    }
}
