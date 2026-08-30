//! Transactional refresh of a linked Project Scene.
//!
//! Sync replaces only the copied source closure and wrapper. Scene identity,
//! manifest storage identity, display name, and every existing placement
//! remain authoritative Project data.

use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use usd_project::{CompositionInspection, ProjectManifestV1, SceneId};
use uuid::Uuid;

use super::{adoption, adoption_authoring, adoption_support, authoring};
use crate::project::source_closure::materialize_source_closure;

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

/// Refresh the linked source transactionally without renaming the Scene.
pub(crate) fn sync_linked_scene_atomic(
    request: LinkedSceneSyncRequest<'_>,
) -> Result<adoption::AdoptedScene> {
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
    let final_source_directory = request
        .project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join("imports")
        .join(SCENES_DIRECTORY)
        .join(request.scene_id.to_string());
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
    fs::create_dir_all(
        temporary_scene_path
            .parent()
            .expect("temporary Scene path has a parent"),
    )?;

    let result = (|| {
        let source_name = materialize_source_closure(
            request.source,
            &temporary_source_directory,
            !request.inspection.dependencies.is_empty(),
        )?;
        adoption_authoring::author_scene_wrapper_to_path(
            &temporary_scene_path,
            request.scene_id,
            &format!(
                "../imports/{SCENES_DIRECTORY}/{}/{source_name}",
                request.scene_id
            ),
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
        fs::rename(&final_source_directory, &old_source_directory)
            .context("stage old linked Scene source closure")?;
        fs::rename(&final_binding_path, &old_binding_path)
            .context("stage old linked source binding")?;

        fs::rename(&temporary_source_directory, &final_source_directory)
            .context("publish refreshed linked Scene source closure")?;
        fs::rename(&temporary_scene_path, &final_scene_path)
            .context("publish refreshed linked Scene layer")?;
        fs::rename(&temporary_binding_path, &final_binding_path)
            .context("publish refreshed linked source binding")?;

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

    if result.is_err() {
        let _ = fs::remove_file(&final_binding_path);
        let _ = fs::remove_file(&final_scene_path);
        let _ = fs::remove_dir_all(&final_source_directory);
        if old_scene_path.exists() {
            let _ = fs::rename(&old_scene_path, &final_scene_path);
        }
        if old_source_directory.exists() {
            let _ = fs::rename(&old_source_directory, &final_source_directory);
        }
        if old_binding_path.exists() {
            let _ = fs::rename(&old_binding_path, &final_binding_path);
        }
    }
    let _ = fs::remove_dir_all(&transaction_directory);
    result
}
