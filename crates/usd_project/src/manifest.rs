use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{ModelId, ModelSourceKind, ProjectRoot, SceneId};

pub const PROJECT_MANIFEST_SCHEMA_VERSION: u32 = 1;

/// Errors raised by the versioned Project manifest contract.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProjectManifestError {
    #[error("unsupported Project manifest schema version {actual}; expected {expected}")]
    UnsupportedSchemaVersion { expected: u32, actual: u32 },
    #[error("Project name must not be empty")]
    EmptyProjectName,
    #[error("invalid storage key: {value}")]
    InvalidStorageKey { value: String },
    #[error("duplicate SceneId in Project manifest: {id}")]
    DuplicateSceneId { id: SceneId },
    #[error("duplicate ModelId in Project manifest: {id}")]
    DuplicateModelId { id: ModelId },
    #[error("duplicate storage key in Project manifest: {value}")]
    DuplicateStorageKey { value: String },
    #[error("Project root SceneId is not registered: {id}")]
    MissingRootScene { id: SceneId },
    #[error("Project root ModelId is not registered: {id}")]
    MissingRootModel { id: ModelId },
}

/// A validated one-component token used for Git-tracked Project storage.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StorageKey(String);

impl StorageKey {
    pub fn new(value: impl Into<String>) -> Result<Self, ProjectManifestError> {
        let value = value.into();
        let drive_relative = value.as_bytes().get(1) == Some(&b':');
        let unsafe_component = value.is_empty()
            || value == "."
            || value == ".."
            || value.contains('/')
            || value.contains('\\')
            || value.contains('\0')
            || drive_relative;

        if unsafe_component {
            return Err(ProjectManifestError::InvalidStorageKey { value });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StorageKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for StorageKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for StorageKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// The canonical Git-tracked Project metadata schema.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectManifestV1 {
    pub schema_version: u32,
    pub project_id: crate::ProjectId,
    pub name: String,
    pub root: ProjectRoot,
    pub scenes: Vec<SceneManifestEntry>,
    pub models: Vec<ModelManifestEntry>,
}

impl ProjectManifestV1 {
    pub fn new(
        project_id: crate::ProjectId,
        name: impl Into<String>,
        root: ProjectRoot,
        scenes: Vec<SceneManifestEntry>,
        models: Vec<ModelManifestEntry>,
    ) -> Self {
        Self {
            schema_version: PROJECT_MANIFEST_SCHEMA_VERSION,
            project_id,
            name: name.into(),
            root,
            scenes,
            models,
        }
    }

    pub fn validate_schema_version(&self) -> Result<(), ProjectManifestError> {
        if self.schema_version != PROJECT_MANIFEST_SCHEMA_VERSION {
            return Err(ProjectManifestError::UnsupportedSchemaVersion {
                expected: PROJECT_MANIFEST_SCHEMA_VERSION,
                actual: self.schema_version,
            });
        }
        Ok(())
    }

    /// Returns a value whose sequence fields have stable ID ordering.
    pub fn canonicalized(&self) -> Self {
        let mut canonical = self.clone();
        canonical.scenes.sort_by_key(|entry| entry.id);
        canonical.models.sort_by_key(|entry| entry.id);
        canonical
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SceneManifestEntry {
    pub id: SceneId,
    pub storage_key: StorageKey,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelManifestEntry {
    pub id: ModelId,
    pub source_kind: ModelSourceKind,
    pub storage_key: StorageKey,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModelId, ProjectId};

    fn storage_key(value: &str) -> StorageKey {
        StorageKey::new(value).unwrap()
    }

    #[test]
    fn canonical_json_is_independent_of_insertion_order() {
        let scene_a = SceneManifestEntry {
            id: SceneId::new_v4(),
            storage_key: storage_key("scene-a"),
        };
        let scene_b = SceneManifestEntry {
            id: SceneId::new_v4(),
            storage_key: storage_key("scene-b"),
        };
        let model_a = ModelManifestEntry {
            id: ModelId::new_v4(),
            source_kind: ModelSourceKind::Usd,
            storage_key: storage_key("model-a"),
        };
        let left = ProjectManifestV1::new(
            ProjectId::new_v4(),
            "Project",
            ProjectRoot::Empty,
            vec![scene_a.clone(), scene_b.clone()],
            vec![model_a.clone()],
        );
        let right = ProjectManifestV1::new(
            left.project_id,
            left.name.clone(),
            ProjectRoot::Empty,
            vec![scene_b, scene_a],
            vec![model_a],
        );

        let left_json = serde_json::to_string_pretty(&left.canonicalized()).unwrap();
        let right_json = serde_json::to_string_pretty(&right.canonicalized()).unwrap();

        assert_eq!(left_json, right_json);
    }

    #[test]
    fn unsafe_storage_keys_are_rejected() {
        for value in [
            "",
            ".",
            "..",
            "/absolute",
            "\\absolute",
            "nested/key",
            "C:drive",
            "nul\0",
        ] {
            assert!(
                StorageKey::new(value).is_err(),
                "accepted unsafe key {value:?}"
            );
        }
    }

    #[test]
    fn schema_version_must_be_exactly_one() {
        let mut manifest = ProjectManifestV1::new(
            ProjectId::new_v4(),
            "Project",
            ProjectRoot::Empty,
            Vec::new(),
            Vec::new(),
        );
        manifest.schema_version = 2;

        assert!(matches!(
            manifest.validate_schema_version(),
            Err(ProjectManifestError::UnsupportedSchemaVersion { actual: 2, .. })
        ));
    }
}
