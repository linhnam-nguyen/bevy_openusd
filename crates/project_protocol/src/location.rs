use serde::{Deserialize, Serialize};

/// The product operation that requested a local Project directory.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectLocationKind {
    CreateProject,
    ImportProject,
    ImportScene,
    ImportModel,
}

/// Opaque process/session-local handle for a host-selected directory.
///
/// The host keeps the corresponding filesystem path private. This token is
/// safe to carry through the frontend without turning a machine-local path
/// into a Project identity or serialized Project DTO.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct LocalSelectionToken(String);

impl LocalSelectionToken {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The user-visible part of a host-selected local directory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LocalSelectionView {
    pub token: LocalSelectionToken,
    pub display_name: String,
}

/// A picker result keeps cancellation out of the error channel.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectLocationResult {
    Selected(LocalSelectionView),
    Cancelled,
}
