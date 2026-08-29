//! Definition-level deletion for Project composition graphs.

use std::{
    fs,
    path::{Path, PathBuf},
};

use project_protocol::{
    ProjectDeleteModelRequest, ProjectDeleteSceneRequest, ProjectModelLifecycleResponse,
    ProjectSceneLifecycleResponse, ProjectWriteError, ProjectWriteErrorCode,
};
use usd_project::{ModelId, ProjectId, SceneId, SceneMemberId};
use uuid::Uuid;

use super::{ProjectApplicationService, ProjectStageMutation};

#[path = "deletion_graph.rs"]
mod deletion_graph;
use deletion_graph::{
    DeleteTarget, DeletionPlan, build_plan, changed_parents, is_deleted_target, read_composition,
};

const PROJECT_METADATA_DIRECTORY: &str = ".usdhub";
const SCENE_SOURCES_DIRECTORY: &str = "imports/scenes";

struct Tombstone {
    original: PathBuf,
    moved: PathBuf,
}

struct ParentBackup {
    original: PathBuf,
    backup: PathBuf,
}

pub(super) fn delete_scene(
    service: &mut ProjectApplicationService,
    request: ProjectDeleteSceneRequest,
) -> Result<ProjectSceneLifecycleResponse, ProjectWriteError> {
    let project_id = request.project_id;
    let scene_id = request.scene_id;
    let _ = delete_definition(service, project_id, DeleteTarget::Scene(scene_id))?;
    Ok(ProjectSceneLifecycleResponse {
        project_id,
        scene_id,
        placement_id: None,
    })
}

pub(super) fn delete_model(
    service: &mut ProjectApplicationService,
    request: ProjectDeleteModelRequest,
) -> Result<ProjectModelLifecycleResponse, ProjectWriteError> {
    let project_id = request.project_id;
    let model_id = request.model_id;
    let result = delete_definition(service, project_id, DeleteTarget::Model(model_id))?;
    Ok(ProjectModelLifecycleResponse {
        project_id,
        model_id,
        placement_ids: result.placement_ids,
    })
}

struct DeletionResult {
    placement_ids: Vec<SceneMemberId>,
}

fn delete_definition(
    service: &mut ProjectApplicationService,
    project_id: ProjectId,
    target: DeleteTarget,
) -> Result<DeletionResult, ProjectWriteError> {
    let (entry, validated) =
        service
            .validated_project(project_id)
            .map_err(|error| ProjectWriteError::Failed {
                code: match error {
                    project_protocol::ProjectReadError::NotFound { .. } => {
                        ProjectWriteErrorCode::ProjectNotFound
                    }
                    _ => ProjectWriteErrorCode::ManifestUnavailable,
                },
            })?;
    match target {
        DeleteTarget::Scene(scene_id) => {
            if validated.scene(scene_id).is_none() {
                return Err(ProjectWriteError::Invalid {
                    code: ProjectWriteErrorCode::SceneNotFound,
                });
            }
            if validated.raw().root == usd_project::ProjectRoot::Scene(scene_id) {
                return Err(ProjectWriteError::Invalid {
                    code: ProjectWriteErrorCode::ProtectedRootScene,
                });
            }
        }
        DeleteTarget::Model(model_id) => {
            if validated.model(model_id).is_none() {
                return Err(ProjectWriteError::Invalid {
                    code: ProjectWriteErrorCode::InvalidSelection,
                });
            }
            if validated.raw().root == usd_project::ProjectRoot::Model(model_id) {
                return Err(ProjectWriteError::Invalid {
                    code: ProjectWriteErrorCode::ProtectedRootScene,
                });
            }
        }
    }

    let project_root = entry.repository_locator();
    let index = read_composition(project_root, &validated)?;
    let plan = build_plan(&index, target);
    let mutation_count = plan.removed_placements.len() + plan.scenes.len() + plan.models.len();
    service
        .stage_mutations
        .ensure_capacity_for(project_root, mutation_count)?;

    let transaction_directory = project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join(".transactions")
        .join(format!("delete-{}", Uuid::new_v4()));
    fs::create_dir_all(&transaction_directory).map_err(|_| delete_error())?;
    let mut backups = Vec::new();
    let mut tombstones = Vec::new();
    let result = (|| {
        let changed_parents = changed_parents(&index, &plan);
        for (ordinal, parent_scene_id) in changed_parents.iter().enumerate() {
            let original =
                crate::project::scene::authoring::scene_path(project_root, *parent_scene_id);
            let backup = transaction_directory
                .join("parents")
                .join(format!("{ordinal}.usda"));
            if let Some(parent) = backup.parent() {
                fs::create_dir_all(parent).map_err(|_| delete_error())?;
            }
            fs::copy(&original, &backup).map_err(|_| delete_error())?;
            backups.push(ParentBackup { original, backup });
        }
        for parent_scene_id in changed_parents {
            let original_members = index
                .members
                .get(&parent_scene_id)
                .expect("changed parent is indexed");
            let remaining = original_members
                .iter()
                .filter(|member| !is_deleted_target(&plan, &member.target))
                .cloned()
                .collect::<Vec<_>>();
            crate::project::scene::authoring::replace_scene_members_atomic(
                &crate::project::scene::authoring::scene_path(project_root, parent_scene_id),
                project_root,
                parent_scene_id,
                &remaining,
            )
            .map_err(|_| delete_error())?;
        }

        for scene_id in &plan.scenes {
            let scene_path = crate::project::scene::authoring::scene_path(project_root, *scene_id);
            tombstones.push(move_to_tombstone(
                &scene_path,
                &transaction_directory,
                &format!("scene-{scene_id}.usda"),
            )?);
            let source_directory = project_root
                .join(PROJECT_METADATA_DIRECTORY)
                .join(SCENE_SOURCES_DIRECTORY)
                .join(scene_id.to_string());
            move_optional_to_tombstone(
                &source_directory,
                &transaction_directory,
                &format!("scene-source-{scene_id}"),
                &mut tombstones,
            )?;
            let binding = crate::project::link::binding_path(project_root, *scene_id);
            move_optional_to_tombstone(
                &binding,
                &transaction_directory,
                &format!("scene-link-{scene_id}.json"),
                &mut tombstones,
            )?;
        }
        for model_id in &plan.models {
            let model_wrapper_path =
                crate::project::model_wrapper::model_wrapper_path(project_root, *model_id);
            let model_directory = model_wrapper_path
                .parent()
                .expect("Model wrapper path has a directory");
            tombstones.push(move_to_tombstone(
                model_directory,
                &transaction_directory,
                &format!("model-{model_id}"),
            )?);
        }

        let previous_manifest = validated.raw().clone();
        let mut next_manifest = previous_manifest.clone();
        next_manifest
            .scenes
            .retain(|scene| !plan.scenes.contains(&scene.id));
        next_manifest
            .models
            .retain(|model| !plan.models.contains(&model.id));
        crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(
            project_root,
            &next_manifest,
        )
        .map_err(|_| delete_error())?;

        for (parent_scene_id, placement_id) in &plan.removed_placements {
            service.stage_mutations.submit_for_project(
                project_root,
                ProjectStageMutation::RemoveScenePlacement {
                    project_id,
                    parent_scene_id: *parent_scene_id,
                    placement_id: *placement_id,
                },
            )?;
        }
        for scene_id in &plan.scenes {
            service.stage_mutations.submit_for_project(
                project_root,
                ProjectStageMutation::DeleteScene {
                    project_id,
                    scene_id: *scene_id,
                },
            )?;
        }
        for model_id in &plan.models {
            service.stage_mutations.submit_for_project(
                project_root,
                ProjectStageMutation::DeleteModel {
                    project_id,
                    model_id: *model_id,
                },
            )?;
        }
        Ok(())
    })();

    if let Err(error) = result {
        restore_backups(&backups);
        restore_tombstones(&tombstones);
        let _ = fs::remove_dir_all(&transaction_directory);
        return Err(error);
    }

    let placement_ids = plan
        .removed_placements
        .iter()
        .map(|(_, placement_id)| *placement_id)
        .collect::<Vec<_>>();
    for scene_id in &plan.scenes {
        let _ = service.cache_warm.remove_target_descriptors(
            project_root,
            &crate::project::cache::ProjectCacheTarget::Scene {
                id: scene_id.to_string(),
            },
        );
    }
    for model_id in &plan.models {
        let _ = service.cache_warm.remove_target_descriptors(
            project_root,
            &crate::project::cache::ProjectCacheTarget::Model {
                id: model_id.to_string(),
            },
        );
    }
    let _ = service.cache_warm.enqueue_affected(
        project_root,
        crate::project::cache::ProjectCacheTarget::ProjectRoot,
    );
    let _ = fs::remove_dir_all(&transaction_directory);
    Ok(DeletionResult { placement_ids })
}

fn move_to_tombstone(
    original: &Path,
    transaction_directory: &Path,
    name: &str,
) -> Result<Tombstone, ProjectWriteError> {
    if !original.exists() {
        return Err(delete_error());
    }
    let moved = transaction_directory.join(name);
    fs::rename(original, &moved).map_err(|_| delete_error())?;
    Ok(Tombstone {
        original: original.to_owned(),
        moved,
    })
}

fn move_optional_to_tombstone(
    original: &Path,
    transaction_directory: &Path,
    name: &str,
    tombstones: &mut Vec<Tombstone>,
) -> Result<(), ProjectWriteError> {
    if original.exists() {
        tombstones.push(move_to_tombstone(original, transaction_directory, name)?);
    }
    Ok(())
}

fn restore_backups(backups: &[ParentBackup]) {
    for backup in backups.iter().rev() {
        let _ = fs::remove_file(&backup.original);
        let _ = fs::rename(&backup.backup, &backup.original);
    }
}

fn restore_tombstones(tombstones: &[Tombstone]) {
    for tombstone in tombstones.iter().rev() {
        let _ = fs::rename(&tombstone.moved, &tombstone.original);
    }
}

fn delete_error() -> ProjectWriteError {
    ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::SceneDeleteFailed,
    }
}
