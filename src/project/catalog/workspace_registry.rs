use std::{
    collections::HashMap,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use usd_project::ProjectId;
use uuid::Uuid;

use super::manifest_store::write_bytes_atomic;

const WORKSPACE_REGISTRY_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct WorkspaceRegistryFile {
    schema_version: u32,
    entries: Vec<WorkspaceProjectEntry>,
}

/// Machine-local project location and recency metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct WorkspaceProjectEntry {
    project_id: ProjectId,
    repository_locator: PathBuf,
    last_opened_ms: Option<u64>,
}

impl WorkspaceProjectEntry {
    pub(crate) fn project_id(&self) -> ProjectId {
        self.project_id
    }

    pub(crate) fn repository_locator(&self) -> &Path {
        &self.repository_locator
    }

    pub(crate) fn last_opened_ms(&self) -> Option<u64> {
        self.last_opened_ms
    }
}

/// Host-owned machine-local registry for known Project repositories.
pub(crate) struct WorkspaceRegistry {
    file_path: PathBuf,
    entries: Vec<WorkspaceProjectEntry>,
    index: HashMap<ProjectId, usize>,
}

impl WorkspaceRegistry {
    pub(crate) fn load(file_path: impl Into<PathBuf>) -> Result<Self> {
        let file_path = file_path.into();
        let bytes = match fs::read(&file_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::empty(file_path)),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("read workspace registry {}", file_path.display()));
            }
        };
        let stored: WorkspaceRegistryFile = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode workspace registry {}", file_path.display()))?;
        if stored.schema_version != WORKSPACE_REGISTRY_SCHEMA_VERSION {
            bail!(
                "unsupported workspace registry schema version {}; expected {}",
                stored.schema_version,
                WORKSPACE_REGISTRY_SCHEMA_VERSION
            );
        }

        let mut registry = Self {
            file_path,
            entries: stored.entries,
            index: HashMap::new(),
        };
        registry.rebuild_index()?;
        Ok(registry)
    }

    pub(crate) fn entries(&self) -> &[WorkspaceProjectEntry] {
        &self.entries
    }

    pub(crate) fn get(&self, project_id: ProjectId) -> Option<&WorkspaceProjectEntry> {
        self.index
            .get(&project_id)
            .map(|index| &self.entries[*index])
    }

    pub(crate) fn register(
        &mut self,
        project_id: ProjectId,
        repository_locator: impl Into<PathBuf>,
        last_opened_ms: Option<u64>,
    ) -> Result<()> {
        let entry = WorkspaceProjectEntry {
            project_id,
            repository_locator: repository_locator.into(),
            last_opened_ms,
        };
        let mut next = self.entries.clone();
        if let Some(index) = self.index.get(&project_id).copied() {
            next[index] = entry;
        } else {
            next.push(entry);
        }
        let next = Self::persist(&self.file_path, next)?;
        self.entries = next;
        self.rebuild_index()
    }

    pub(crate) fn remove(&mut self, project_id: ProjectId) -> Result<bool> {
        let Some(index) = self.index.get(&project_id).copied() else {
            return Ok(false);
        };
        let mut next = self.entries.clone();
        next.remove(index);
        let next = Self::persist(&self.file_path, next)?;
        self.entries = next;
        self.rebuild_index()?;
        Ok(true)
    }

    fn empty(file_path: PathBuf) -> Self {
        Self {
            file_path,
            entries: Vec::new(),
            index: HashMap::new(),
        }
    }

    fn rebuild_index(&mut self) -> Result<()> {
        self.index.clear();
        self.index.reserve(self.entries.len());
        for (index, entry) in self.entries.iter().enumerate() {
            if self.index.insert(entry.project_id, index).is_some() {
                bail!(
                    "duplicate ProjectId in workspace registry: {}",
                    entry.project_id
                );
            }
        }
        Ok(())
    }

    fn persist(
        file_path: &Path,
        mut entries: Vec<WorkspaceProjectEntry>,
    ) -> Result<Vec<WorkspaceProjectEntry>> {
        entries.sort_by_key(|entry| entry.project_id);
        let stored = WorkspaceRegistryFile {
            schema_version: WORKSPACE_REGISTRY_SCHEMA_VERSION,
            entries: entries.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&stored).context("serialize workspace registry")?;
        if let Some(parent) = file_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).with_context(|| {
                format!("create workspace registry directory {}", parent.display())
            })?;
        }
        let temporary_path = file_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .join(format!(".workspace-registry.{}.tmp", Uuid::new_v4()));
        write_bytes_atomic(&temporary_path, file_path, &bytes)?;
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use usd_project::{
        ProjectCapabilities, ProjectContentCounts, ProjectRoot, ProjectSummary, RepositorySummary,
    };

    use super::*;

    #[test]
    fn register_update_remove_and_restart_preserve_one_entry_per_project() {
        let directory = tempdir().unwrap();
        let registry_path = directory.path().join("workspace.json");
        let first = ProjectId::new_v4();
        let second = ProjectId::new_v4();
        let first_locator = directory.path().join("first");
        let updated_locator = directory.path().join("first-updated");

        let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
        registry.register(first, &first_locator, Some(10)).unwrap();
        registry
            .register(second, directory.path().join("second"), None)
            .unwrap();
        registry
            .register(first, &updated_locator, Some(20))
            .unwrap();

        assert_eq!(registry.entries().len(), 2);
        assert_eq!(
            registry.get(first).unwrap().repository_locator(),
            updated_locator
        );
        assert_eq!(registry.get(first).unwrap().last_opened_ms(), Some(20));

        assert!(registry.remove(second).unwrap());
        assert!(!registry.remove(second).unwrap());

        let restarted = WorkspaceRegistry::load(&registry_path).unwrap();
        assert_eq!(restarted.entries().len(), 1);
        assert_eq!(
            restarted.get(first).unwrap().repository_locator(),
            updated_locator
        );
        assert!(
            fs::read_to_string(registry_path)
                .unwrap()
                .contains("schema_version")
        );
    }

    #[test]
    fn duplicate_project_ids_are_rejected_on_load() {
        let directory = tempdir().unwrap();
        let project_id = ProjectId::new_v4();
        let stored = WorkspaceRegistryFile {
            schema_version: WORKSPACE_REGISTRY_SCHEMA_VERSION,
            entries: vec![
                WorkspaceProjectEntry {
                    project_id,
                    repository_locator: PathBuf::from("one"),
                    last_opened_ms: None,
                },
                WorkspaceProjectEntry {
                    project_id,
                    repository_locator: PathBuf::from("two"),
                    last_opened_ms: None,
                },
            ],
        };
        fs::write(
            directory.path().join("workspace.json"),
            serde_json::to_vec(&stored).unwrap(),
        )
        .unwrap();

        assert!(WorkspaceRegistry::load(directory.path().join("workspace.json")).is_err());
    }

    #[test]
    fn project_summary_serialization_does_not_include_machine_local_locator() {
        let local_locator = "/machine-local/private/project";
        let summary = ProjectSummary {
            id: ProjectId::new_v4(),
            name: "Project".to_owned(),
            root: ProjectRoot::Empty,
            repository: RepositorySummary {
                active_branch: None,
                branches: Vec::new(),
                dirty: false,
                head: None,
                latest_commit: None,
            },
            counts: ProjectContentCounts::default(),
            issues: usd_project::ProjectIssueSummary::default(),
            capabilities: ProjectCapabilities::default(),
        };

        let encoded = serde_json::to_string(&summary).unwrap();
        assert!(!encoded.contains(local_locator));
    }
}
