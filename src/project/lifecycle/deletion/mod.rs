//! Definition-level deletion for Project composition graphs.

use std::{
    fs,
    path::{Path, PathBuf},
};

use project_protocol::{
    ProjectDeleteModelRequest, ProjectDeleteSceneRequest, ProjectModelLifecycleResponse,
    ProjectSceneLifecycleResponse, ProjectWriteError, ProjectWriteErrorCode,
};
use usd_project::{ProjectId, SceneMemberId};
use uuid::Uuid;

use crate::project::service::{ProjectApplicationService, ProjectStageMutation};

mod graph;
use graph::{DeleteTarget, build_plan, changed_parents, is_deleted_target, read_composition};

const PROJECT_METADATA_DIRECTORY: &str = ".usdhub";

struct Tombstone {
    original: PathBuf,
    moved: PathBuf,
}

struct ParentBackup {
    original: PathBuf,
    backup: PathBuf,
}

pub(crate) fn delete_scene(
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

pub(crate) fn delete_model(
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
    let previous_manifest = validated.raw().clone();
    let index = read_composition(project_root, &validated)?;
    let plan = build_plan(&index, target);
    let changed_parent_scene_ids = changed_parents(&index, &plan);
    let mutation_count = plan.removed_placements.len() + plan.scenes.len() + plan.models.len();
    service
        .stage_mutations
        .ensure_capacity_for(project_root, mutation_count)?;

    let transaction_directory = project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join(".transactions")
        .join(format!("delete-{}", Uuid::new_v4()));
    fs::create_dir_all(&transaction_directory).map_err(|_| delete_error())?;
    let ancestor_preimages = crate::project::scene::timeline::capture_ancestor_preimages(
        project_root, &previous_manifest, &changed_parent_scene_ids,
        &transaction_directory.join("timeline-preimages"),
    ).map_err(|error| { log::error!("capture deletion timeline preimages: {error:#}"); delete_error() })?;
    let manifest_path = crate::project::catalog::manifest_store::manifest_path(project_root);
    let manifest_backup = transaction_directory.join("manifest-preimage.json");
    fs::copy(&manifest_path, &manifest_backup).map_err(|_| delete_error())?;
    let mut backups = Vec::new();
    let mut tombstones = Vec::new();
    let mut mutations = Vec::with_capacity(mutation_count);
    mutations.extend(
        plan.removed_placements
            .iter()
            .map(
                |(parent_scene_id, placement_id)| ProjectStageMutation::RemoveScenePlacement {
                    project_id,
                    parent_scene_id: *parent_scene_id,
                    placement_id: *placement_id,
                },
            ),
    );
    mutations.extend(
        plan.scenes
            .iter()
            .map(|scene_id| ProjectStageMutation::DeleteScene {
                project_id,
                scene_id: *scene_id,
            }),
    );
    mutations.extend(
        plan.models
            .iter()
            .map(|model_id| ProjectStageMutation::DeleteModel {
                project_id,
                model_id: *model_id,
            }),
    );
    let result = (|| {
        for (ordinal, parent_scene_id) in changed_parent_scene_ids.iter().enumerate() {
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
        let mut candidate_manifest = previous_manifest.clone();
        candidate_manifest.scenes.retain(|scene| !plan.scenes.contains(&scene.id));
        candidate_manifest.models.retain(|model| !plan.models.contains(&model.id));
        let mut replacements = std::collections::HashMap::new();
        for parent_scene_id in &changed_parent_scene_ids {
            if plan.scenes.contains(parent_scene_id) { continue; }
            let original_members = index.members.get(parent_scene_id).ok_or_else(delete_error)?;
            let remaining = original_members.iter()
                .filter(|member| !is_deleted_target(&plan, &member.target)).cloned().collect::<Vec<_>>();
            replacements.insert(*parent_scene_id, remaining);
        }
        let prepared = crate::project::scene::timeline::prepare_scene_member_timelines(
            project_root, &candidate_manifest, &replacements,
            &transaction_directory.join("staged-timeline-layers"),
        ).map_err(|error| { log::error!("prepare deletion timelines: {error:#}"); delete_error() })?;
        let updated_ancestors = prepared.publish().map_err(|error| {
            log::error!("publish deletion timelines: {error:#}"); delete_error()
        })?;

        for scene_id in &plan.scenes {
            let scene_path = crate::project::scene::authoring::scene_path(project_root, *scene_id);
            tombstones.push(move_to_tombstone(
                &scene_path,
                &transaction_directory,
                &format!("scene-{scene_id}.usda"),
            )?);
            let source_directory = crate::project::storage::ProjectStorageLayout::new(project_root)
                .readable_scene_import_dir(*scene_id);
            move_optional_to_tombstone(
                &source_directory,
                &transaction_directory,
                &format!("scene-source-{scene_id}"),
                &mut tombstones,
            )?;
            let binding = crate::project::models::link::binding_path(project_root, *scene_id);
            move_optional_to_tombstone(
                &binding,
                &transaction_directory,
                &format!("scene-link-{scene_id}.json"),
                &mut tombstones,
            )?;
        }
        for model_id in &plan.models {
            let model_wrapper_path =
                crate::project::models::wrapper::model_wrapper_path(project_root, *model_id);
            let model_directory = model_wrapper_path
                .parent()
                .expect("Model wrapper path has a directory");
            tombstones.push(move_to_tombstone(
                model_directory,
                &transaction_directory,
                &format!("model-{model_id}"),
            )?);
        }

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

        service
            .stage_mutations
            .submit_batch_for_project(project_root, &mutations)?;
        Ok(updated_ancestors)
    })();

    let updated_ancestors = match result {
        Ok(updated_ancestors) => updated_ancestors,
        Err(error) => {
            let manifest_restored = crate::project::scene::timeline::restore_timeline_preimage(&manifest_path, &manifest_backup);
            let parents_restored = restore_backups(&backups);
            let definitions_restored = restore_tombstones(&tombstones);
            let ancestors_restored = crate::project::scene::timeline::restore_ancestor_preimages(&ancestor_preimages);
            let failures: Vec<_> = [manifest_restored, parents_restored, definitions_restored, ancestors_restored]
                .into_iter().filter_map(|result| result.err()).collect();
            if failures.is_empty() {
                let _ = fs::remove_dir_all(&transaction_directory);
            } else {
                log::error!("deletion recovery required at {}: {failures:?}", transaction_directory.display());
            }
            return Err(error);
        }
    };

    let placement_ids = plan
        .removed_placements
        .iter()
        .map(|(_, placement_id)| *placement_id)
        .collect::<Vec<_>>();
    let mut cleanup_failed = false;
    for scene_id in &plan.scenes {
        if !service.cache_warm.remove_target_descriptors(
            project_root,
            &crate::cache::ProjectCacheTarget::Scene {
                id: scene_id.to_string(),
            },
        ) {
            cleanup_failed = true;
        }
    }
    for model_id in &plan.models {
        if !service.cache_warm.remove_target_descriptors(
            project_root,
            &crate::cache::ProjectCacheTarget::Model {
                id: model_id.to_string(),
            },
        ) {
            cleanup_failed = true;
        }
    }
    let mut cache_targets = vec![crate::cache::ProjectCacheTarget::ProjectRoot];
    cache_targets.extend(changed_parent_scene_ids.iter().map(|scene_id| {
        crate::cache::ProjectCacheTarget::Scene {
            id: scene_id.to_string(),
        }
    }));
    cache_targets.extend(updated_ancestors.iter().map(|scene_id| {
        crate::cache::ProjectCacheTarget::Scene {
            id: scene_id.to_string(),
        }
    }));
    if service
        .cache_warm
        .enqueue_targets_for_mutation(project_root, cache_targets)
        .is_err()
    {
        cleanup_failed = true;
    }
    if fs::remove_dir_all(&transaction_directory).is_err() {
        cleanup_failed = true;
    }
    if cleanup_failed {
        return Err(delete_error());
    }
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

fn restore_backups(backups: &[ParentBackup]) -> anyhow::Result<()> {
    let mut failures = Vec::new();
    for backup in backups.iter().rev() {
        if let Err(error) = crate::project::scene::timeline::restore_timeline_preimage(&backup.original, &backup.backup) {
            failures.push(format!("{}: {error:#}", backup.original.display()));
        }
    }
    anyhow::ensure!(failures.is_empty(), "parent recovery required: {}", failures.join("; "));
    Ok(())
}

fn restore_tombstones(tombstones: &[Tombstone]) -> anyhow::Result<()> {
    let mut failures = Vec::new();
    for tombstone in tombstones.iter().rev() {
        if let Err(error) = fs::rename(&tombstone.moved, &tombstone.original) {
            failures.push(format!("{}: {error}", tombstone.original.display()));
        }
    }
    anyhow::ensure!(failures.is_empty(), "definition recovery required: {}", failures.join("; "));
    Ok(())
}

fn delete_error() -> ProjectWriteError {
    ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::SceneDeleteFailed,
    }
}
