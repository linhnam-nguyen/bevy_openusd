//! Transactional adoption of a composed USD source as a Project Scene.
//!
//! The adapter keeps the source layer untouched. It authors a small USDHub
//! wrapper that references the source, validates it before publish, and
//! publishes the manifest only after all new Scene files are ready.

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{adoption_authoring, adoption_support, authoring};
use crate::project::catalog::manifest_store::ManifestStore;
use crate::project::source_closure::materialize_source_closure;
use crate::project::storage::ProjectStorageLayout;
use anyhow::{Context, Result, bail, ensure};
use usd_project::{
    CompositionInspection, ProjectManifestV1, ProjectRoot, SceneCompositionGraph, SceneId,
    SceneManifestEntry, SceneMember, ScenePlacementTransform, StorageKey,
};
use uuid::Uuid;

const PROJECT_METADATA_DIRECTORY: &str = ".usdhub";
const TRANSACTIONS_DIRECTORY: &str = ".transactions";
const SCENES_DIRECTORY: &str = "scenes";

/// Inputs for one backend-only Scene adoption transaction.
pub(crate) struct SceneAdoptionRequest<'a> {
    pub project_root: &'a Path,
    pub source: &'a Path,
    pub inspection: &'a CompositionInspection,
    pub name: &'a str,
    pub base_manifest: &'a ProjectManifestV1,
    pub graph: &'a SceneCompositionGraph,
    pub parent_scene_id: Option<SceneId>,
    /// Complete known parent membership when the parent is being updated.
    pub parent_members: &'a [SceneMember],
    /// When set, place an existing Scene identity instead of allocating one.
    pub target_scene_id: Option<SceneId>,
    pub set_as_root: bool,
    pub placement: ScenePlacementTransform,
    /// When set, publish a private source binding for the synchronized copy.
    pub linked_source: Option<&'a Path>,
}

/// The identities and manifest proposed by a successful adoption.
#[derive(Clone, Debug)]
pub(crate) struct AdoptedScene {
    pub scene_id: SceneId,
    pub member: Option<SceneMember>,
    pub scene_path: PathBuf,
    pub manifest: ProjectManifestV1,
}

/// Prepare and publish one composed Scene adoption transaction.
pub(crate) fn adopt_scene_atomic(request: SceneAdoptionRequest<'_>) -> Result<AdoptedScene> {
    request
        .base_manifest
        .validate()
        .context("validate base Project manifest")?;
    adoption_support::ensure_adoptable(request.inspection)?;
    adoption_support::ensure_current_manifest(request.project_root, request.base_manifest)?;

    let default_prim = adoption_support::revalidate_source(request.source, request.inspection)?;
    let scene_name = request.name.trim();
    ensure!(
        !scene_name.is_empty(),
        "adopted Scene name must not be empty"
    );
    let storage_key = StorageKey::new(scene_name.to_owned())?;
    let (scene_id, scene_is_new) = match request.target_scene_id {
        Some(scene_id) => {
            ensure!(
                request
                    .base_manifest
                    .scenes
                    .iter()
                    .any(|entry| entry.id == scene_id),
                "existing Scene target is not registered in the Project manifest"
            );
            ensure!(
                authoring::scene_path(request.project_root, scene_id).exists(),
                "existing Scene target has no canonical Scene layer"
            );
            (scene_id, false)
        }
        None => (SceneId::new_v4(), true),
    };

    if request.parent_scene_id.is_none() {
        ensure!(
            request.parent_members.is_empty(),
            "parent members require a parent Scene"
        );
    }
    let (parent_members, member) = if let Some(parent_scene_id) = request.parent_scene_id {
        ensure!(
            request
                .base_manifest
                .scenes
                .iter()
                .any(|entry| entry.id == parent_scene_id),
            "parent Scene is not registered in the Project manifest"
        );
        let (_, member) = adoption_support::propose_scene_placement_with_name(
            request.graph,
            parent_scene_id,
            scene_id,
            scene_name,
            request.placement,
        )?;
        let mut parent_members = request.parent_members.to_vec();
        parent_members.push(member.clone());
        (Some(parent_members), Some(member))
    } else {
        (None, None)
    };

    let mut manifest_candidate = request.base_manifest.clone();
    if scene_is_new {
        manifest_candidate.scenes.push(SceneManifestEntry {
            id: scene_id,
            storage_key,
            display_name: scene_name.to_owned(),
        });
    }
    if request.set_as_root {
        manifest_candidate.root = ProjectRoot::Scene(scene_id);
    }
    manifest_candidate
        .validate()
        .context("validate proposed Project manifest")?;

    let metadata_directory = request.project_root.join(PROJECT_METADATA_DIRECTORY);
    let transaction_directory = metadata_directory
        .join(TRANSACTIONS_DIRECTORY)
        .join(Uuid::new_v4().to_string());
    let temporary_scene_directory = transaction_directory.join(SCENES_DIRECTORY);
    fs::create_dir_all(&temporary_scene_directory)
        .context("create Scene adoption transaction directory")?;

    let temporary_scene_path = temporary_scene_directory.join(format!("{scene_id}.usda"));
    let temporary_source_directory = transaction_directory
        .join("imports")
        .join(SCENES_DIRECTORY)
        .join(scene_id.to_string());
    let temporary_binding_path = transaction_directory.join("linked-source.json");
    let final_scene_path = if scene_is_new {
        let scene = manifest_candidate
            .scenes
            .iter()
            .find(|entry| entry.id == scene_id)
            .expect("new Scene manifest entry exists");
        authoring::scene_path_for_entry(
            request.project_root,
            scene,
            manifest_candidate.root == ProjectRoot::Scene(scene_id),
        )
    } else {
        authoring::scene_path(request.project_root, scene_id)
    };
    let final_source_directory =
        ProjectStorageLayout::new(request.project_root).canonical_scene_import_dir(scene_id);
    let parent_scene_path = request
        .parent_scene_id
        .map(|parent| authoring::scene_path(request.project_root, parent));
    let temporary_parent_path = request
        .parent_scene_id
        .map(|parent| temporary_scene_directory.join(format!("{parent}.usda")));
    let parent_backup_path = transaction_directory.join("parent-backup.usda");
    let mut parent_published = false;
    let mut scene_published = false;
    let mut source_published = false;
    let mut parent_backup_created = false;
    let mut binding_published = false;
    let final_binding_path = crate::project::link::binding_path(request.project_root, scene_id);

    let result = (|| {
        if scene_is_new {
            let source_name =
                materialize_source_closure(request.source, &temporary_source_directory)?;
            adoption_authoring::author_scene_wrapper_to_path(
                &temporary_scene_path,
                request.project_root,
                &final_scene_path,
                scene_id,
                &final_source_directory.join(&source_name),
                &default_prim,
                scene_name,
                &request.inspection.spatial,
                request.linked_source.is_some(),
            )?;
            adoption_authoring::validate_scene_wrapper(
                &temporary_scene_path,
                scene_id,
                &request.inspection.spatial,
                request.linked_source.is_some(),
            )?;
            ensure!(
                !final_scene_path.exists(),
                "new Project Scene canonical layer already exists"
            );
            if let Some(linked_source) = request.linked_source {
                crate::project::link::prepare_binding(
                    &temporary_binding_path,
                    scene_id,
                    linked_source,
                )?;
            }
        }

        if let (Some(parent_scene_path), Some(temporary_parent_path), Some(parent_members)) = (
            parent_scene_path.as_ref(),
            temporary_parent_path.as_ref(),
            parent_members.as_deref(),
        ) {
            if scene_is_new {
                adoption_authoring::prepare_parent_layer_with_scene_path(
                    parent_scene_path,
                    temporary_parent_path,
                    request.project_root,
                    request
                        .parent_scene_id
                        .expect("parent path implies parent identity"),
                    parent_members,
                    scene_id,
                    &final_scene_path,
                )?;
            } else {
                adoption_authoring::prepare_parent_layer(
                    parent_scene_path,
                    temporary_parent_path,
                    request.project_root,
                    request
                        .parent_scene_id
                        .expect("parent path implies parent identity"),
                    parent_members,
                )?;
            }
            authoring::validate_scene_file(
                temporary_parent_path,
                request
                    .parent_scene_id
                    .expect("parent path implies parent identity"),
                parent_members,
            )?;
        }

        if let Some(parent_scene_path) = parent_scene_path.as_ref() {
            if parent_scene_path.exists() {
                fs::copy(parent_scene_path, &parent_backup_path)
                    .context("backup existing parent Scene layer")?;
                parent_backup_created = true;
            }
            fs::rename(
                temporary_parent_path
                    .as_ref()
                    .expect("parent temp path exists"),
                parent_scene_path,
            )
            .context("publish updated parent Scene layer")?;
            parent_published = true;
        }

        if scene_is_new {
            if final_source_directory.exists() {
                bail!("canonical Project Scene source directory already exists");
            }
            if let Some(parent) = final_source_directory.parent() {
                fs::create_dir_all(parent)
                    .context("create canonical Project Scene source directory")?;
            }
            fs::rename(&temporary_source_directory, &final_source_directory)
                .context("publish adopted Project Scene source closure")?;
            source_published = true;
            if let Some(parent) = final_scene_path.parent() {
                fs::create_dir_all(parent).context("create canonical Project Scene directory")?;
            }
            fs::rename(&temporary_scene_path, &final_scene_path)
                .context("publish adopted Project Scene layer")?;
            scene_published = true;
        }

        if request.linked_source.is_some() {
            ensure!(
                !final_binding_path.exists(),
                "canonical linked source binding already exists"
            );
            if let Some(parent) = final_binding_path.parent() {
                fs::create_dir_all(parent).context("create linked source binding directory")?;
            }
            fs::rename(&temporary_binding_path, &final_binding_path)
                .context("publish linked source binding")?;
            binding_published = true;
        }

        if manifest_candidate != request.base_manifest.canonicalized() {
            ManifestStore::write_manifest_atomic(request.project_root, &manifest_candidate)
                .context("publish adopted Project manifest")?;
        }
        Ok(())
    })();

    let final_result = match result {
        Ok(()) => Ok(AdoptedScene {
            scene_id,
            member,
            scene_path: final_scene_path.clone(),
            manifest: manifest_candidate,
        }),
        Err(error) => {
            if let Err(rollback_error) = adoption_support::rollback_publication(
                parent_scene_path.as_deref(),
                &parent_backup_path,
                parent_backup_created,
                parent_published,
                &final_scene_path,
                scene_published,
                &final_source_directory,
                source_published,
                &final_binding_path,
                binding_published,
            ) {
                Err(error.context(format!(
                    "rollback Scene adoption publication: {rollback_error}"
                )))
            } else {
                Err(error)
            }
        }
    };
    let _ = fs::remove_dir_all(&transaction_directory);
    final_result
}

pub(crate) use crate::project::scene::linked_sync::{
    LinkedSceneSyncRequest, sync_linked_scene_atomic,
};

/// Propose a placement while preserving the target Scene identity.
pub(crate) fn propose_scene_placement(
    graph: &SceneCompositionGraph,
    parent_scene_id: SceneId,
    target_scene_id: SceneId,
) -> Result<(SceneCompositionGraph, SceneMember)> {
    adoption_support::propose_scene_placement_with_name(
        graph,
        parent_scene_id,
        target_scene_id,
        "",
        ScenePlacementTransform::IDENTITY,
    )
}

#[cfg(test)]
#[path = "adoption_tests.rs"]
mod adoption_tests;
