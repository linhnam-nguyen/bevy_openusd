//! Project-owned runtime-cache identity primitives.
//!
//! Cache state is derived acceleration data. Reuse is keyed by the canonical
//! composition closure of the requested Project target. Git baseline facts
//! remain available for diagnostics, while disposable cache and recovery
//! files are excluded from the broader working-tree fingerprint.

use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use usd_git::GitRepository;

use super::storage::{CACHE_DIRECTORY, PROJECT_METADATA_DIRECTORY, RECOVERY_DIRECTORY};

#[path = "cache_descriptor.rs"]
mod descriptor;
#[path = "cache_target.rs"]
mod target;

pub(crate) use descriptor::{
    ProjectCacheDescriptor, ProjectCacheIdentity, ProjectCacheState, ProjectCacheStore,
    ProjectCacheTarget,
};
pub(crate) use target::target_content_hash;

/// Source identity used by a Project runtime-cache descriptor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum ProjectCacheSourceStamp {
    /// The committed Git tree is authoritative for a clean worktree.
    GitCommit { oid: String },
    /// Dirty and unborn worktrees are identified by canonical content.
    WorkingTree { fingerprint: String },
}

/// Git and source facts captured together for cache identity and diagnostics.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ProjectCacheBaseline {
    pub(crate) branch: Option<String>,
    pub(crate) head: Option<String>,
    pub(crate) dirty: bool,
    pub(crate) source: ProjectCacheSourceStamp,
}

/// Read the authoritative Git state without touching the working tree.
pub(crate) fn inspect_git_baseline(project_root: &Path) -> Result<ProjectCacheBaseline> {
    let repository = usd_git::Repository::open(project_root)
        .with_context(|| format!("open Project repository {}", project_root.display()))?;
    let branch = repository
        .current_branch()
        .context("read current Project branch")?;
    let head = repository
        .head()
        .context("read current Project HEAD")?
        .map(|revision| revision.id().to_string());
    let dirty = repository
        .working_tree_status()
        .context("read Project working-tree status")?
        .dirty;
    let source = source_stamp(dirty, head.as_deref(), fingerprint_project(project_root)?);
    Ok(ProjectCacheBaseline {
        branch,
        head,
        dirty,
        source,
    })
}

fn source_stamp(dirty: bool, head: Option<&str>, fingerprint: String) -> ProjectCacheSourceStamp {
    match (dirty, head) {
        (false, Some(oid)) => ProjectCacheSourceStamp::GitCommit {
            oid: oid.to_owned(),
        },
        _ => ProjectCacheSourceStamp::WorkingTree { fingerprint },
    }
}

/// Fingerprint canonical Project content while ignoring disposable local
/// state.  Names and file boundaries are included so concatenation cannot
/// produce the same digest as a different file tree.
pub(crate) fn fingerprint_project(project_root: &Path) -> Result<String> {
    let root = fs::canonicalize(project_root)
        .with_context(|| format!("canonicalize Project root {}", project_root.display()))?;
    let mut files = Vec::new();
    collect_files(&root, &root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

    let mut hasher = blake3::Hasher::new();
    for (relative, kind, bytes) in files {
        hasher.update(kind.as_bytes());
        hasher.update(relative.as_bytes());
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn collect_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, String, Vec<u8>)>,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("read Project directory {}", directory.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("relativize Project path {}", path.display()))?;
        if is_disposable_path(relative) {
            continue;
        }
        let relative = relative.to_string_lossy().replace('\\', "/");
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("read Project metadata {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&path)
                .with_context(|| format!("read Project symlink {}", path.display()))?;
            files.push((
                relative,
                "symlink".to_owned(),
                target.to_string_lossy().into_owned().into_bytes(),
            ));
        } else if metadata.is_dir() {
            collect_files(root, &path, files)?;
        } else if metadata.is_file() {
            files.push((relative, "file".to_owned(), fs::read(&path)?));
        } else {
            bail!("unsupported Project filesystem entry {}", path.display());
        }
    }
    Ok(())
}

fn is_disposable_path(relative: &Path) -> bool {
    let mut components = relative.components();
    let Some(std::path::Component::Normal(first)) = components.next() else {
        return false;
    };
    if first == ".git" {
        return true;
    }
    matches!(
        (components.next(), components.next()),
        (
            Some(std::path::Component::Normal(child)),
            _
        ) if first == PROJECT_METADATA_DIRECTORY
            && (child == CACHE_DIRECTORY || child == RECOVERY_DIRECTORY)
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use tempfile::tempdir;

    #[test]
    fn unborn_repository_uses_working_tree_fingerprint() -> Result<()> {
        let directory = tempdir()?;
        usd_git::Repository::init(directory.path())?;
        fs::write(directory.path().join("scene.usda"), b"scene")?;

        let first = inspect_git_baseline(directory.path())?;
        assert!(first.head.is_none());
        assert!(first.dirty);
        assert!(matches!(
            &first.source,
            ProjectCacheSourceStamp::WorkingTree { .. }
        ));

        fs::write(directory.path().join("scene.usda"), b"changed")?;
        let second = inspect_git_baseline(directory.path())?;
        assert_ne!(first.source, second.source);
        Ok(())
    }

    #[test]
    fn disposable_cache_and_recovery_do_not_change_working_fingerprint() -> Result<()> {
        let directory = tempdir()?;
        usd_git::Repository::init(directory.path())?;
        fs::write(directory.path().join("scene.usda"), b"scene")?;
        let first = fingerprint_project(directory.path())?;

        fs::create_dir_all(directory.path().join(".usdhub/cache/objects"))?;
        fs::create_dir_all(directory.path().join(".usdhub/recovery"))?;
        fs::write(
            directory.path().join(".usdhub/cache/objects/object.blob"),
            b"derived",
        )?;
        fs::write(
            directory.path().join(".usdhub/recovery/pending"),
            b"derived",
        )?;

        assert_eq!(fingerprint_project(directory.path())?, first);
        Ok(())
    }

    #[test]
    fn clean_head_uses_commit_oid() {
        assert_eq!(
            source_stamp(false, Some("abc123"), "working".to_owned()),
            ProjectCacheSourceStamp::GitCommit {
                oid: "abc123".to_owned()
            }
        );
    }
}
