//! Target-scoped source closure hashing for Project runtime caches.

use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};

use super::ProjectCacheTarget;

struct TargetHashEntry {
    relative: String,
    kind: String,
    path: Option<PathBuf>,
    inline: Vec<u8>,
}

/// Hash only the canonical files that compose one Project target.
///
/// This is deliberately narrower than [`super::cache::fingerprint_project`].
/// A Scene cache includes its authored Scene layer and recursively referenced
/// Scene/Model targets; a Model cache includes its wrapper and materialized
/// source closure. Unrelated siblings therefore keep their reusable
/// descriptors.
pub(crate) fn target_content_hash(
    project_root: &Path,
    target: &ProjectCacheTarget,
) -> Result<usd_model::HashDigest> {
    let manifest =
        crate::project::catalog::manifest_store::ManifestStore::read_validated(project_root)
            .context("read Project manifest for target cache identity")?;
    let root = fs::canonicalize(project_root)
        .with_context(|| format!("canonicalize Project root {}", project_root.display()))?;
    let mut files = Vec::new();
    let mut visited = HashSet::new();
    collect_target_files(&root, &manifest, target, &mut visited, &mut files)?;
    files.sort_by(|left, right| {
        left.relative
            .cmp(&right.relative)
            .then_with(|| left.kind.cmp(&right.kind))
    });

    let mut hasher = blake3::Hasher::new();
    hasher.update(b"usdhub-project-target-closure-v1");
    for entry in files {
        hasher.update(entry.kind.as_bytes());
        hasher.update(entry.relative.as_bytes());
        if let Some(path) = entry.path {
            let metadata = fs::metadata(&path)
                .with_context(|| format!("read Project target metadata {}", path.display()))?;
            hasher.update(&metadata.len().to_le_bytes());
            let mut file = fs::File::open(&path)
                .with_context(|| format!("open Project target {}", path.display()))?;
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let read = file
                    .read(&mut buffer)
                    .with_context(|| format!("read Project target {}", path.display()))?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
        } else {
            hasher.update(&(entry.inline.len() as u64).to_le_bytes());
            hasher.update(&entry.inline);
        }
    }
    Ok(usd_model::HashDigest::new(*hasher.finalize().as_bytes()))
}

fn collect_target_files(
    project_root: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
    target: &ProjectCacheTarget,
    visited: &mut HashSet<String>,
    files: &mut Vec<TargetHashEntry>,
) -> Result<()> {
    if !visited.insert(target.key()) {
        return Ok(());
    }
    files.push(TargetHashEntry {
        relative: format!("@target/{}", target.key()),
        kind: "target".to_owned(),
        path: None,
        inline: Vec::new(),
    });
    if let Some(name) = target_display_name(manifest, target) {
        files.push(TargetHashEntry {
            relative: format!("@name/{}", target.key()),
            kind: "name".to_owned(),
            path: None,
            inline: name.into_bytes(),
        });
    }
    match target {
        ProjectCacheTarget::ProjectRoot => match &manifest.raw().root {
            usd_project::ProjectRoot::Empty => {}
            usd_project::ProjectRoot::Scene(id) => collect_target_files(
                project_root,
                manifest,
                &ProjectCacheTarget::Scene { id: id.to_string() },
                visited,
                files,
            )?,
            usd_project::ProjectRoot::Model(id) => collect_target_files(
                project_root,
                manifest,
                &ProjectCacheTarget::Model { id: id.to_string() },
                visited,
                files,
            )?,
        },
        ProjectCacheTarget::Scene { id } => {
            let scene = manifest
                .scenes()
                .iter()
                .find(|scene| scene.id.to_string() == *id)
                .with_context(|| format!("Scene cache target {id} is not in the manifest"))?;
            let path = crate::project::scene::authoring::scene_path(project_root, scene.id);
            collect_one_file(project_root, &path, files)?;
            let imported_directory =
                crate::project::storage::ProjectStorageLayout::new(project_root)
                    .readable_scene_import_dir(scene.id);
            match fs::symlink_metadata(&imported_directory) {
                Ok(metadata) => {
                    ensure!(
                        metadata.is_dir() && !metadata.file_type().is_symlink(),
                        "Project imported Scene closure must be a regular directory: {}",
                        imported_directory.display()
                    );
                    collect_target_directory(project_root, &imported_directory, files)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "read imported Scene closure {}",
                            imported_directory.display()
                        )
                    });
                }
            }
            for member in crate::project::scene::authoring::read_scene_members(&path, scene.id)? {
                let child = match member.target {
                    usd_project::SceneMemberTarget::Scene(child) => ProjectCacheTarget::Scene {
                        id: child.to_string(),
                    },
                    usd_project::SceneMemberTarget::Model(model) => ProjectCacheTarget::Model {
                        id: model.to_string(),
                    },
                };
                collect_target_files(project_root, manifest, &child, visited, files)?;
            }
        }
        ProjectCacheTarget::Model { id } => {
            let model = manifest
                .models()
                .iter()
                .find(|model| model.id.to_string() == *id)
                .with_context(|| format!("Model cache target {id} is not in the manifest"))?;
            let wrapper = crate::project::model_wrapper::model_wrapper_path(project_root, model.id);
            let directory = wrapper
                .parent()
                .context("canonical Model wrapper has no parent directory")?;
            collect_target_directory(project_root, directory, files)?;
        }
    }
    Ok(())
}

fn target_display_name(
    manifest: &usd_project::ValidatedProjectManifest,
    target: &ProjectCacheTarget,
) -> Option<String> {
    match target {
        ProjectCacheTarget::ProjectRoot => Some(manifest.raw().name.clone()),
        ProjectCacheTarget::Scene { id } => manifest
            .scenes()
            .iter()
            .find(|scene| scene.id.to_string() == *id)
            .map(|scene| scene.display_name.clone()),
        ProjectCacheTarget::Model { id } => manifest
            .models()
            .iter()
            .find(|model| model.id.to_string() == *id)
            .map(|model| model.display_name.clone()),
    }
}

fn collect_one_file(
    project_root: &Path,
    path: &Path,
    files: &mut Vec<TargetHashEntry>,
) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("read Project target metadata {}", path.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "Project target must be a regular non-symlink file: {}",
        path.display()
    );
    let relative = path
        .strip_prefix(project_root)
        .with_context(|| format!("relativize Project target {}", path.display()))?;
    files.push(TargetHashEntry {
        relative: relative.to_string_lossy().replace('\\', "/"),
        kind: "file".to_owned(),
        path: Some(path.to_path_buf()),
        inline: Vec::new(),
    });
    Ok(())
}

fn collect_target_directory(
    root: &Path,
    directory: &Path,
    files: &mut Vec<TargetHashEntry>,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("read Project target directory {}", directory.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("relativize Project target path {}", path.display()))?
            .to_string_lossy()
            .replace('\\', "/");
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("read Project target metadata {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&path)
                .with_context(|| format!("read Project target symlink {}", path.display()))?;
            files.push(TargetHashEntry {
                relative,
                kind: "symlink".to_owned(),
                path: None,
                inline: target.to_string_lossy().into_owned().into_bytes(),
            });
        } else if metadata.is_dir() {
            collect_target_directory(root, &path, files)?;
        } else if metadata.is_file() {
            files.push(TargetHashEntry {
                relative,
                kind: "file".to_owned(),
                path: Some(path),
                inline: Vec::new(),
            });
        } else {
            bail!(
                "unsupported Project target filesystem entry {}",
                path.display()
            );
        }
    }
    Ok(())
}
