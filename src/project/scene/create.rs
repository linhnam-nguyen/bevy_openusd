//! Transactional authoring of a new Project Scene.

use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use usd_project::{
    ProjectManifestV1, ProjectRoot, SceneCompositionGraph, SceneId, SceneManifestEntry,
    SceneMember, SceneMemberId, SceneMemberTarget, StorageKey,
};
use uuid::Uuid;

use super::{adoption_authoring, authoring};
use crate::project::catalog::manifest_store::ManifestStore;

const PROJECT_METADATA_DIRECTORY: &str = ".usdhub";
const TRANSACTIONS_DIRECTORY: &str = ".transactions";
const SCENES_DIRECTORY: &str = "scenes";

pub(crate) struct CreateSceneRequest<'a> {
    pub project_root: &'a Path,
    pub base_manifest: &'a ProjectManifestV1,
    pub graph: &'a SceneCompositionGraph,
    pub parent_scene_id: Option<SceneId>,
    pub parent_members: &'a [SceneMember],
    pub storage_key: StorageKey,
    pub set_as_root: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct CreatedScene {
    pub scene_id: SceneId,
    pub member: Option<SceneMember>,
    pub manifest: ProjectManifestV1,
}

pub(crate) fn create_scene_atomic(request: CreateSceneRequest<'_>) -> Result<CreatedScene> {
    request
        .base_manifest
        .validate()
        .context("validate base Project manifest")?;
    if request.parent_scene_id.is_none() {
        ensure!(
            request.parent_members.is_empty(),
            "parent members require a parent Scene"
        );
    }

    let scene_id = SceneId::new_v4();
    let display_name = request.storage_key.to_string();
    let (parent_members, member) = if let Some(parent_scene_id) = request.parent_scene_id {
        ensure!(
            request
                .base_manifest
                .scenes
                .iter()
                .any(|entry| entry.id == parent_scene_id),
            "parent Scene is not registered in the Project manifest"
        );
        let mut proposed_graph = request.graph.clone();
        proposed_graph
            .add_placement(parent_scene_id, scene_id)
            .context("validate proposed Scene placement")?;
        let member = SceneMember {
            id: SceneMemberId::new_v4(),
            target: SceneMemberTarget::Scene(scene_id),
            name: Some(display_name.clone()),
            transform: Default::default(),
        };
        let mut parent_members = request.parent_members.to_vec();
        parent_members.push(member.clone());
        (Some(parent_members), Some(member))
    } else {
        (None, None)
    };

    let mut manifest = request.base_manifest.clone();
    manifest.scenes.push(SceneManifestEntry {
        id: scene_id,
        display_name: display_name.clone(),
        storage_key: request.storage_key,
    });
    if request.set_as_root {
        manifest.root = ProjectRoot::Scene(scene_id);
    }
    manifest
        .validate()
        .context("validate proposed Project manifest")?;

    let transaction_directory = request
        .project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join(TRANSACTIONS_DIRECTORY)
        .join(Uuid::new_v4().to_string());
    let temporary_scene_directory = transaction_directory.join(SCENES_DIRECTORY);
    fs::create_dir_all(&temporary_scene_directory)
        .context("create Scene creation transaction directory")?;

    let temporary_scene_path = temporary_scene_directory.join(format!("{scene_id}.usda"));
    let final_scene_path = authoring::scene_path(request.project_root, scene_id);
    let parent_scene_path = request
        .parent_scene_id
        .map(|parent| authoring::scene_path(request.project_root, parent));
    let temporary_parent_path = request
        .parent_scene_id
        .map(|parent| temporary_scene_directory.join(format!("{parent}.usda")));
    let parent_backup_path = transaction_directory.join("parent-backup.usda");
    let mut parent_published = false;
    let mut scene_published = false;
    let mut parent_backup_created = false;

    let result = (|| {
        ensure!(
            !final_scene_path.exists(),
            "new Project Scene canonical layer already exists"
        );
        let stage = authoring::new_scene_stage_with_name(scene_id, &display_name)?;
        stage
            .root_layer()
            .export(temporary_scene_path.to_string_lossy().as_ref())
            .context("export temporary Project Scene layer")?;
        authoring::validate_scene_file(&temporary_scene_path, scene_id, &[])?;

        if let (Some(parent_path), Some(temporary_parent_path), Some(parent_members)) = (
            parent_scene_path.as_ref(),
            temporary_parent_path.as_ref(),
            parent_members.as_deref(),
        ) {
            adoption_authoring::prepare_parent_layer(
                parent_path,
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

        if let Some(parent_path) = parent_scene_path.as_ref() {
            if parent_path.exists() {
                fs::copy(parent_path, &parent_backup_path)
                    .context("backup existing parent Scene layer")?;
                parent_backup_created = true;
            }
            fs::rename(
                temporary_parent_path
                    .as_ref()
                    .expect("parent temp path exists"),
                parent_path,
            )
            .context("publish updated parent Scene layer")?;
            parent_published = true;
        }

        if let Some(parent) = final_scene_path.parent() {
            fs::create_dir_all(parent).context("create canonical Project Scene directory")?;
        }
        fs::rename(&temporary_scene_path, &final_scene_path)
            .context("publish new Project Scene layer")?;
        scene_published = true;

        ManifestStore::write_manifest_atomic(request.project_root, &manifest)
            .context("publish Project manifest after Scene creation")?;
        Ok(())
    })();

    let final_result = match result {
        Ok(()) => Ok(CreatedScene {
            scene_id,
            member,
            manifest,
        }),
        Err(error) => {
            if let Err(rollback_error) = rollback_publication(
                parent_scene_path.as_deref(),
                &parent_backup_path,
                parent_backup_created,
                parent_published,
                &final_scene_path,
                scene_published,
            ) {
                Err(error.context(format!(
                    "rollback Scene creation publication: {rollback_error}"
                )))
            } else {
                Err(error)
            }
        }
    };
    let _ = fs::remove_dir_all(&transaction_directory);
    final_result
}

fn rollback_publication(
    parent_path: Option<&Path>,
    backup_path: &Path,
    backup_created: bool,
    parent_published: bool,
    scene_path: &Path,
    scene_published: bool,
) -> Result<()> {
    if scene_published {
        fs::remove_file(scene_path).context("remove newly published Scene layer")?;
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
