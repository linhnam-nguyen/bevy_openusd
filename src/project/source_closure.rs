//! Exact dependency closure handling for Project Scene and Model imports.

use std::{
    collections::HashSet,
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};

use super::cache_contract::ProjectCacheTarget;

#[path = "source_closure_discovery.rs"]
mod discovery;
#[path = "source_closure_io.rs"]
mod io;
#[path = "source_closure_localize.rs"]
mod localize;

pub(crate) use discovery::{LocalizedDependencyReport, discover, open_stage_with_resolver};
pub(crate) use localize::{
    materialize_source_closure, materialize_source_closure_with_resolver,
    source_closure_fingerprint,
};

/// Discover a canonical Project asset and prove its complete dependency
/// closure remains inside the Project root. The discovery itself walks
/// parsed OpenUSD layer fields, references, payloads, and asset values; this
/// boundary adds the Storage v2 containment invariant.
pub(crate) fn dependency_containment_report(
    project_root: &Path,
    root_asset: &Path,
) -> Result<LocalizedDependencyReport> {
    let project_root = std::fs::canonicalize(project_root)
        .with_context(|| format!("canonicalize Project root {}", project_root.display()))?;
    let report = discover(root_asset)?;
    ensure!(
        report.unresolved.is_empty(),
        "canonical Project dependency closure has unresolved assets: {:?}",
        report.unresolved
    );
    for dependency in report
        .layers
        .iter()
        .chain(report.non_layer_assets.iter())
        .chain(std::iter::once(&report.root_asset))
    {
        ensure!(
            dependency.starts_with(&project_root),
            "canonical Project dependency escapes the Project root: {}",
            dependency.display()
        );
    }
    Ok(report)
}

// Target-scoped canonical source-closure hashing for Project runtime caches.
struct TargetHashEntry {
    relative: String,
    kind: String,
    path: Option<PathBuf>,
    inline: Vec<u8>,
    presentation_filtered: bool,
}

/// Hash only the canonical files that compose one Project target.
///
/// This is deliberately narrower than [`super::cache::fingerprint_project`].
/// A Scene identity owns its authored layer plus its imported source closure,
/// but not referenced child Scene/Model payloads or presentation-only names.
/// A Model identity owns its wrapper plus imported source closure.
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
    hasher.update(b"usdhub-project-target-closure-v2");
    for entry in files {
        hasher.update(entry.kind.as_bytes());
        hasher.update(entry.relative.as_bytes());
        if let Some(path) = entry.path {
            if entry.presentation_filtered {
                hash_managed_layer(&path, &mut hasher)?;
            } else {
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
                    if read == 0 { break; }
                    hasher.update(&buffer[..read]);
                }
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
        presentation_filtered: false,
    });
    if matches!(target, ProjectCacheTarget::ProjectRoot) {
        files.push(TargetHashEntry {
            relative: "@name/project".to_owned(),
            kind: "name".to_owned(),
            path: None,
            inline: manifest.raw().name.as_bytes().to_vec(),
            presentation_filtered: false,
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
            collect_one_file(project_root, &path, files, true)?;
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
        }
        ProjectCacheTarget::Model { id } => {
            let model = manifest
                .models()
                .iter()
                .find(|model| model.id.to_string() == *id)
                .with_context(|| format!("Model cache target {id} is not in the manifest"))?;
            let wrapper = crate::project::model_wrapper::model_wrapper_path(project_root, model.id);
            collect_one_file(project_root, &wrapper, files, true)?;
            let imported_directory = crate::project::storage::ProjectStorageLayout::new(project_root)
                .readable_model_import_dir(model.id);
            if imported_directory.exists() {
                collect_target_directory(project_root, &imported_directory, files)?;
            }
        }
    }
    Ok(())
}

fn collect_one_file(
    project_root: &Path,
    path: &Path,
    files: &mut Vec<TargetHashEntry>,
    presentation_filtered: bool,
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
        presentation_filtered,
    });
    Ok(())
}

fn hash_managed_layer(path: &Path, hasher: &mut blake3::Hasher) -> Result<()> {
    let file = fs::File::open(path)
        .with_context(|| format!("open managed Project target {}", path.display()))?;
    for line in BufReader::new(file).split(b'\n') {
        let line = line.with_context(|| format!("read managed Project target {}", path.display()))?;
        let mut start = 0;
        while start < line.len() && matches!(line[start], b' ' | b'\t') { start += 1; }
        if line[start..].starts_with(b"string ui:displayName =") { continue; }
        hasher.update(&(line.len() as u64).to_le_bytes());
        hasher.update(&line);
    }
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
                presentation_filtered: false,
            });
        } else if metadata.is_dir() {
            collect_target_directory(root, &path, files)?;
        } else if metadata.is_file() {
            files.push(TargetHashEntry {
                relative,
                kind: "file".to_owned(),
                path: Some(path),
                inline: Vec::new(),
                presentation_filtered: false,
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

#[cfg(test)]
#[path = "source_closure_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "source_closure_pattern_tests.rs"]
mod pattern_tests;

#[cfg(test)]
#[path = "source_closure_optional_tests.rs"]
mod optional_tests;
