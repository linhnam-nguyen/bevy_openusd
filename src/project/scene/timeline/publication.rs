//! Durable staged publication, preimage capture, and verified rollback for
//! canonical timeline layers.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use usd_project::{ProjectManifestV1, SceneId};
use uuid::Uuid;

use super::propagation::{affected_ancestors_child_first, all_scenes_child_first};
use crate::project::scene::authoring::{
    read_scene_members, scene_path, sync_parent_best_effort,
};

/// Flush file data to disk using a write handle so Windows `FlushFileBuffers`
/// succeeds without `ERROR_ACCESS_DENIED`.
pub(crate) fn sync_file_durable(path: &Path) -> Result<()> {
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .with_context(|| format!("open {} for durability sync", path.display()))?
        .sync_all()
        .with_context(|| format!("sync {}", path.display()))
}

/// Capture ancestor preimages before an enclosing canonical mutation starts.
/// The enclosing transaction owns this directory until its commit or verified
/// rollback. Restoration never infers old metadata from new USD.
pub(crate) fn capture_ancestor_preimages(
    project_root: &Path,
    manifest: &ProjectManifestV1,
    seeds: &[SceneId],
    directory: &Path,
) -> Result<Vec<(PathBuf, PathBuf)>> {
    if seeds.is_empty() {
        return Ok(Vec::new());
    }
    fs::create_dir_all(directory)?;
    let mut graph = HashMap::with_capacity(manifest.scenes.len());
    for scene in &manifest.scenes {
        let path = scene_path(project_root, scene.id);
        if !path.is_file() && seeds.contains(&scene.id) {
            graph.insert(scene.id, Vec::new());
            continue;
        }
        ensure!(
            path.is_file(),
            "required Scene layer {} is missing: {}",
            scene.id,
            path.display()
        );
        graph.insert(scene.id, read_scene_members(&path, scene.id)?);
    }
    all_scenes_child_first(&graph, manifest)?;
    let ancestors = affected_ancestors_child_first(&graph, seeds)?;
    let mut backups = Vec::new();
    for (index, id) in ancestors.into_iter().enumerate() {
        let original = scene_path(project_root, id);
        let backup = directory.join(format!("ancestor-{index}.usda"));
        fs::copy(&original, &backup)
            .with_context(|| format!("capture ancestor {id} preimage"))?;
        sync_file_durable(&backup)?;
        backups.push((original, backup));
    }
    let journal = directory.join("ancestor-preimages.json");
    fs::write(&journal, serde_json::to_vec_pretty(&backups)?)?;
    sync_file_durable(&journal)?;
    Ok(backups)
}

pub(crate) fn restore_ancestor_preimages(backups: &[(PathBuf, PathBuf)]) -> Result<()> {
    let mut failures = Vec::new();
    for (original, backup) in backups.iter().rev() {
        if let Err(error) = restore_timeline_preimage(original, backup) {
            failures.push(format!("{}: {error:#}", original.display()));
        }
    }
    ensure!(
        failures.is_empty(),
        "ancestor timeline recovery required: {}",
        failures.join("; ")
    );
    Ok(())
}

pub(crate) fn publish_staged_layers_atomic<K>(
    transaction_dir: &Path,
    staged: &[(K, PathBuf, PathBuf)],
) -> Result<()> {
    let mut backups: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(staged.len());
    for (ordinal, (_, _, final_path)) in staged.iter().enumerate() {
        let backup_path = transaction_dir.join(format!("backup-{ordinal}.usda"));
        fs::copy(final_path, &backup_path).with_context(|| {
            format!(
                "backup canonical layer {} before timeline publish",
                final_path.display()
            )
        })?;
        sync_file_durable(&backup_path)?;
        backups.push((final_path.clone(), backup_path));
    }
    // Keep an explicit destination/preimage map. If publication or restoration
    // fails, the transaction directory remains available to the recovery owner.
    let journal = transaction_dir.join("timeline-preimages.json");
    fs::write(&journal, serde_json::to_vec_pretty(&backups)?)?;
    sync_file_durable(&journal)?;
    let mut published_count = 0usize;
    for (_, temp_path, final_path) in staged {
        let publish_result = sync_file_durable(temp_path).and_then(|()| {
            fs::rename(temp_path, final_path).context("replace canonical timeline layer")
        });
        if let Err(error) = publish_result {
            let mut restoration_errors = Vec::new();
            for (original, backup) in backups.iter().take(published_count).rev() {
                // Copy through a sibling temporary file and replace directly.
                // Never remove the only canonical copy or consume the backup.
                if let Err(restore_error) = restore_timeline_preimage(original, backup) {
                    restoration_errors.push(format!("{}: {restore_error:#}", original.display()));
                }
            }
            if !restoration_errors.is_empty() {
                bail!(
                    "timeline publication failed at {}: {error}; recovery required at {}: {}",
                    final_path.display(),
                    transaction_dir.display(),
                    restoration_errors.join("; ")
                );
            }
            return Err(error).with_context(|| {
                format!(
                    "publish updated timeline layer {} (rolled back)",
                    final_path.display()
                )
            });
        }
        published_count += 1;
        sync_parent_best_effort(final_path.parent());
    }
    Ok(())
}

pub(crate) fn restore_timeline_preimage(original: &Path, backup: &Path) -> Result<()> {
    let sibling = original.with_file_name(format!(".timeline-restore-{}.tmp", Uuid::new_v4()));
    fs::copy(backup, &sibling).context("copy timeline recovery preimage")?;
    sync_file_durable(&sibling)?;
    fs::rename(&sibling, original).context("replace canonical layer with recovery preimage")?;
    sync_parent_best_effort(original.parent());
    ensure!(
        fs::read(original)? == fs::read(backup)?,
        "timeline preimage restoration verification failed"
    );
    Ok(())
}
