//! Transactional adoption of a composed USD source as a Project Scene.
//!
//! The adapter keeps the source layer untouched. It authors a small USDHub
//! wrapper that references the source, validates it before publish, and
//! publishes the manifest only after all new Scene files are ready.

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{adoption_authoring, authoring, inspection::inspect_composition};
use crate::project::catalog::manifest_store::ManifestStore;
use crate::project::source_closure::materialize_source_closure;
use anyhow::{Context, Result, bail, ensure};
use openusd::usd::{InitialLoadSet, Stage};
use usd_project::{
    CompositionClassification, CompositionInspection, ProjectManifestV1, ProjectRoot,
    SceneCompositionGraph, SceneId, SceneManifestEntry, SceneMember, SceneMemberId,
    SceneMemberTarget, StorageKey,
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
    ensure_adoptable(request.inspection)?;
    ensure_current_manifest(request.project_root, request.base_manifest)?;

    let default_prim = revalidate_source(request.source, request.inspection)?;
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
        let (_, member) = propose_scene_placement_with_name(
            request.graph,
            parent_scene_id,
            scene_id,
            scene_name,
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
    let final_scene_path = authoring::scene_path(request.project_root, scene_id);
    let final_source_directory = request
        .project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join("imports")
        .join(SCENES_DIRECTORY)
        .join(scene_id.to_string());
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

    let result = (|| {
        if scene_is_new {
            let source_name = materialize_source_closure(
                request.source,
                &temporary_source_directory,
                !request.inspection.dependencies.is_empty(),
            )?;
            adoption_authoring::author_scene_wrapper_to_path(
                &temporary_scene_path,
                scene_id,
                &format!("../imports/{SCENES_DIRECTORY}/{scene_id}/{source_name}"),
                &default_prim,
                scene_name,
                &request.inspection.spatial,
            )?;
            adoption_authoring::validate_scene_wrapper(
                &temporary_scene_path,
                scene_id,
                &request.inspection.spatial,
            )?;
            ensure!(
                !final_scene_path.exists(),
                "new Project Scene canonical layer already exists"
            );
        }

        if let (Some(parent_scene_path), Some(temporary_parent_path), Some(parent_members)) = (
            parent_scene_path.as_ref(),
            temporary_parent_path.as_ref(),
            parent_members.as_deref(),
        ) {
            adoption_authoring::prepare_parent_layer(
                parent_scene_path,
                temporary_parent_path,
                request.project_root,
                request
                    .parent_scene_id
                    .expect("parent path implies parent identity"),
                parent_members,
            )?;
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
            if let Err(rollback_error) = rollback_publication(
                parent_scene_path.as_deref(),
                &parent_backup_path,
                parent_backup_created,
                parent_published,
                &final_scene_path,
                scene_published,
                &final_source_directory,
                source_published,
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

/// Propose a placement while preserving the target Scene identity.
pub(crate) fn propose_scene_placement(
    graph: &SceneCompositionGraph,
    parent_scene_id: SceneId,
    target_scene_id: SceneId,
) -> Result<(SceneCompositionGraph, SceneMember)> {
    propose_scene_placement_with_name(graph, parent_scene_id, target_scene_id, "")
}

fn propose_scene_placement_with_name(
    graph: &SceneCompositionGraph,
    parent_scene_id: SceneId,
    target_scene_id: SceneId,
    name: &str,
) -> Result<(SceneCompositionGraph, SceneMember)> {
    let mut proposed_graph = graph.clone();
    proposed_graph
        .add_placement(parent_scene_id, target_scene_id)
        .context("validate proposed Scene placement")?;
    Ok((
        proposed_graph,
        SceneMember {
            id: SceneMemberId::new_v4(),
            target: SceneMemberTarget::Scene(target_scene_id),
            name: (!name.is_empty()).then(|| name.to_owned()),
            transform: Default::default(),
        },
    ))
}

fn ensure_adoptable(inspection: &CompositionInspection) -> Result<()> {
    ensure!(
        matches!(
            inspection.classification,
            CompositionClassification::NativeUsdHubScene | CompositionClassification::SceneLike
        ),
        "USD source is not an eligible Scene adoption candidate"
    );
    ensure!(
        inspection.dependencies.iter().all(|dependency| {
            !matches!(
                dependency.classification,
                usd_project::DependencyClassification::Missing
                    | usd_project::DependencyClassification::Unsupported
            )
        }),
        "USD source has unresolved or unsupported composition dependencies"
    );
    Ok(())
}

fn ensure_current_manifest(project_root: &Path, expected: &ProjectManifestV1) -> Result<()> {
    let current = ManifestStore::read_validated(project_root)
        .context("read current Project manifest before Scene adoption")?;
    ensure!(
        current.raw() == &expected.canonicalized(),
        "Project manifest changed after the adoption candidate was inspected"
    );
    Ok(())
}

fn revalidate_source(source: &Path, expected: &CompositionInspection) -> Result<String> {
    ensure!(
        source.is_file(),
        "Scene adoption source disappeared or is not a file"
    );
    let actual = inspect_composition(source).context("reinspect Scene adoption source")?;
    ensure!(
        &actual == expected,
        "Scene adoption source changed after inspection"
    );
    ensure_adoptable(&actual)?;

    let source_string = source
        .to_str()
        .context("Scene adoption source path must be valid UTF-8")?;
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(source_string)
        .context("reopen Scene adoption source")?;
    stage
        .default_prim()
        .map(|token| token.as_str().to_owned())
        .context("Scene adoption source has no defaultPrim")
}

fn rollback_publication(
    parent_path: Option<&Path>,
    backup_path: &Path,
    backup_created: bool,
    parent_published: bool,
    scene_path: &Path,
    scene_published: bool,
    source_path: &Path,
    source_published: bool,
) -> Result<()> {
    if scene_published {
        fs::remove_file(scene_path).context("remove newly published Scene layer")?;
    }
    if source_published {
        fs::remove_dir_all(source_path).context("remove newly published Scene source closure")?;
    }
    if parent_published {
        let parent_path = parent_path.context("published parent Scene path is missing")?;
        if backup_created {
            fs::remove_file(parent_path).context("remove replaced parent Scene layer")?;
            fs::rename(backup_path, parent_path).context("restore parent Scene layer backup")?;
        } else {
            fs::remove_file(parent_path).context("remove newly published parent Scene layer")?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "adoption_tests.rs"]
mod adoption_tests;
