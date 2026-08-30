//! Transactional refresh of a linked Project Scene.
//!
//! Sync replaces only the copied source closure and wrapper. Scene identity,
//! manifest storage identity, display name, and every existing placement
//! remain authoritative Project data.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use usd_project::{CompositionInspection, ProjectManifestV1, SceneId};
use uuid::Uuid;

use super::{adoption, adoption_authoring, adoption_support, authoring};
use crate::project::source_closure::materialize_source_closure;
use crate::project::storage::ProjectStorageLayout;

const PROJECT_METADATA_DIRECTORY: &str = ".usdhub";
const TRANSACTIONS_DIRECTORY: &str = ".transactions";
const SCENES_DIRECTORY: &str = "scenes";

/// Inputs for refreshing one linked Scene while preserving its Project
/// identity and placement graph.
pub(crate) struct LinkedSceneSyncRequest<'a> {
    pub project_root: &'a Path,
    pub source: &'a Path,
    pub inspection: &'a CompositionInspection,
    pub scene_id: SceneId,
    pub base_manifest: &'a ProjectManifestV1,
}

/// Published files remain recoverable until the stage-refresh outbox accepts
/// its handoff. The service either finalizes this publication or rolls it
/// back, so a failed refresh publication cannot leave a half-synced Scene.
pub(crate) struct LinkedSceneSyncPublication {
    transaction_directory: PathBuf,
    final_scene_path: PathBuf,
    final_source_directory: PathBuf,
    final_binding_path: PathBuf,
    old_scene_path: PathBuf,
    old_source_directory: PathBuf,
    old_binding_path: PathBuf,
    complete: bool,
}

impl LinkedSceneSyncPublication {
    pub(crate) fn finalize(mut self) -> Result<()> {
        self.complete = true;
        fs::remove_dir_all(&self.transaction_directory)
            .context("remove completed linked Scene transaction")
    }

    pub(crate) fn rollback(&mut self) -> Result<()> {
        if self.complete {
            return Ok(());
        }
        let mut first_error = None;
        if self.final_binding_path.exists() {
            if let Err(error) = fs::remove_file(&self.final_binding_path) {
                first_error.get_or_insert(error);
            }
        }
        if self.final_scene_path.exists() {
            if let Err(error) = fs::remove_file(&self.final_scene_path) {
                first_error.get_or_insert(error);
            }
        }
        if self.final_source_directory.exists() {
            if let Err(error) = fs::remove_dir_all(&self.final_source_directory) {
                first_error.get_or_insert(error);
            }
        }
        if self.old_scene_path.exists() {
            if let Err(error) = fs::rename(&self.old_scene_path, &self.final_scene_path) {
                first_error.get_or_insert(error);
            }
        }
        if self.old_source_directory.exists() {
            if let Err(error) = fs::rename(&self.old_source_directory, &self.final_source_directory)
            {
                first_error.get_or_insert(error);
            }
        }
        if self.old_binding_path.exists() {
            if let Err(error) = fs::rename(&self.old_binding_path, &self.final_binding_path) {
                first_error.get_or_insert(error);
            }
        }
        let cleanup = fs::remove_dir_all(&self.transaction_directory);
        self.complete = true;
        if let Some(error) = first_error {
            return Err(error.into());
        }
        cleanup.context("remove rolled-back linked Scene transaction")
    }
}

/// Refresh the linked source transactionally without renaming the Scene.
pub(crate) fn sync_linked_scene_atomic(
    request: LinkedSceneSyncRequest<'_>,
) -> Result<(adoption::AdoptedScene, LinkedSceneSyncPublication)> {
    request
        .base_manifest
        .validate()
        .context("validate base Project manifest")?;
    adoption_support::ensure_adoptable(request.inspection)?;
    adoption_support::ensure_current_manifest(request.project_root, request.base_manifest)?;
    let default_prim = adoption_support::revalidate_source(request.source, request.inspection)?;
    let scene_name = request
        .base_manifest
        .scenes
        .iter()
        .find(|entry| entry.id == request.scene_id)
        .map(|entry| entry.display_name.as_str())
        .ok_or_else(|| anyhow::anyhow!("linked Scene target is not registered"))?;
    ensure!(!scene_name.trim().is_empty(), "linked Scene name is empty");

    let final_scene_path = authoring::scene_path(request.project_root, request.scene_id);
    ensure!(
        final_scene_path.is_file(),
        "linked Scene target has no canonical Scene layer"
    );
    let final_source_directory = ProjectStorageLayout::new(request.project_root)
        .canonical_scene_import_dir(request.scene_id);
    let final_binding_path =
        crate::project::link::binding_path(request.project_root, request.scene_id);
    ensure!(
        final_binding_path.is_file(),
        "Scene is not backed by a linked source binding"
    );

    let transaction_directory = request
        .project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join(TRANSACTIONS_DIRECTORY)
        .join(format!("sync-{}", Uuid::new_v4()));
    let temporary_scene_path = transaction_directory
        .join(SCENES_DIRECTORY)
        .join(format!("{}.usda", request.scene_id));
    let temporary_source_directory = transaction_directory
        .join("imports")
        .join(SCENES_DIRECTORY)
        .join(request.scene_id.to_string());
    let temporary_binding_path = transaction_directory.join("linked-source.json");
    let old_scene_path = transaction_directory.join("old-scene.usda");
    let old_source_directory = transaction_directory.join("old-source");
    let old_binding_path = transaction_directory.join("old-binding.json");
    let mut old_scene_staged = false;
    let mut old_source_staged = false;
    let mut old_binding_staged = false;
    let mut new_source_published = false;
    let mut new_scene_published = false;
    let mut new_binding_published = false;
    fs::create_dir_all(
        temporary_scene_path
            .parent()
            .expect("temporary Scene path has a parent"),
    )?;

    let result = (|| {
        let source_name = materialize_source_closure(request.source, &temporary_source_directory)?;
        adoption_authoring::author_scene_wrapper_to_path(
            &temporary_scene_path,
            request.project_root,
            &final_scene_path,
            request.scene_id,
            &final_source_directory.join(&source_name),
            &default_prim,
            scene_name,
            &request.inspection.spatial,
        )?;
        adoption_authoring::validate_scene_wrapper(
            &temporary_scene_path,
            request.scene_id,
            &request.inspection.spatial,
        )?;
        crate::project::link::prepare_binding(
            &temporary_binding_path,
            request.scene_id,
            request.source,
        )?;

        fs::rename(&final_scene_path, &old_scene_path).context("stage old linked Scene layer")?;
        old_scene_staged = true;
        fs::rename(&final_source_directory, &old_source_directory)
            .context("stage old linked Scene source closure")?;
        old_source_staged = true;
        fs::rename(&final_binding_path, &old_binding_path)
            .context("stage old linked source binding")?;
        old_binding_staged = true;

        fs::rename(&temporary_source_directory, &final_source_directory)
            .context("publish refreshed linked Scene source closure")?;
        new_source_published = true;
        fs::rename(&temporary_scene_path, &final_scene_path)
            .context("publish refreshed linked Scene layer")?;
        new_scene_published = true;
        fs::rename(&temporary_binding_path, &final_binding_path)
            .context("publish refreshed linked source binding")?;
        new_binding_published = true;

        let manifest_candidate = request.base_manifest.clone();
        manifest_candidate
            .validate()
            .context("validate refreshed Scene manifest")?;
        Ok(adoption::AdoptedScene {
            scene_id: request.scene_id,
            member: None,
            scene_path: final_scene_path.clone(),
            manifest: manifest_candidate,
        })
    })();

    let adopted = match result {
        Ok(adopted) => adopted,
        Err(error) => {
            if new_binding_published {
                let _ = fs::remove_file(&final_binding_path);
            }
            if new_scene_published {
                let _ = fs::remove_file(&final_scene_path);
            }
            if new_source_published {
                let _ = fs::remove_dir_all(&final_source_directory);
            }
            if old_scene_staged {
                let _ = fs::rename(&old_scene_path, &final_scene_path);
            }
            if old_source_staged {
                let _ = fs::rename(&old_source_directory, &final_source_directory);
            }
            if old_binding_staged {
                let _ = fs::rename(&old_binding_path, &final_binding_path);
            }
            let _ = fs::remove_dir_all(&transaction_directory);
            return Err(error);
        }
    };
    Ok((
        adopted,
        LinkedSceneSyncPublication {
            transaction_directory,
            final_scene_path,
            final_source_directory,
            final_binding_path,
            old_scene_path,
            old_source_directory,
            old_binding_path,
            complete: false,
        },
    ))
}
