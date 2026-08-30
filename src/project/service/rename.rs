//! Authoritative rename transactions for Project-managed content.

use std::{
    fs,
    path::{Path, PathBuf},
};

#[cfg(test)]
use std::cell::Cell;

use project_protocol::{
    ProjectRenameResponse, ProjectWriteError, ProjectWriteErrorCode, ProjectWriteTarget,
};
use usd_project::{ProjectRoot, SceneMemberTarget};
use uuid::Uuid;

use super::ProjectApplicationService;
use crate::project::catalog::manifest_store::ManifestStore;

#[cfg(test)]
thread_local! {
    static FAIL_AFTER_PLACEMENT: Cell<Option<usize>> = const { Cell::new(None) };
}

#[cfg(test)]
fn set_test_failure_after_placement(index: usize) {
    FAIL_AFTER_PLACEMENT.with(|failure| failure.set(Some(index)));
}

#[cfg(test)]
fn clear_test_failure_after_placement() {
    FAIL_AFTER_PLACEMENT.with(|failure| failure.set(None));
}

#[cfg(test)]
fn should_fail_after_placement(index: usize) -> bool {
    FAIL_AFTER_PLACEMENT.with(|failure| failure.get() == Some(index))
}

pub(super) fn rename(
    service: &mut ProjectApplicationService,
    project_id: usd_project::ProjectId,
    target: ProjectWriteTarget,
    requested_name: &str,
) -> Result<ProjectRenameResponse, ProjectWriteError> {
    let name = requested_name.trim();
    if name.is_empty() {
        return Err(ProjectWriteError::Invalid {
            code: match target {
                ProjectWriteTarget::Project(_) => ProjectWriteErrorCode::InvalidProjectName,
                ProjectWriteTarget::Scene(_) => ProjectWriteErrorCode::InvalidSceneName,
                ProjectWriteTarget::Model(_) => ProjectWriteErrorCode::InvalidModelName,
            },
        });
    }

    let publisher = service.publication_coordinator.publisher(project_id);
    let _publication = publisher
        .lock()
        .expect("Project publication lock is not poisoned");
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
    let project_root = entry.repository_locator();
    let mut next_manifest = validated.raw().clone();
    let mut placement_paths = Vec::new();

    match target {
        ProjectWriteTarget::Project(target_project_id) if target_project_id == project_id => {
            next_manifest.name = name.to_owned();
            let ProjectRoot::Scene(root_scene_id) = next_manifest.root else {
                return Err(ProjectWriteError::Invalid {
                    code: ProjectWriteErrorCode::InvalidRootForComposition,
                });
            };
            let root_entry = next_manifest
                .scenes
                .iter_mut()
                .find(|scene| scene.id == root_scene_id)
                .ok_or(ProjectWriteError::Invalid {
                    code: ProjectWriteErrorCode::ProtectedRootScene,
                })?;
            root_entry.display_name = name.to_owned();
            placement_paths.push(crate::project::scene::authoring::scene_path(
                project_root,
                root_scene_id,
            ));
        }
        ProjectWriteTarget::Project(_) => {
            return Err(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::InvalidSelection,
            });
        }
        ProjectWriteTarget::Scene(scene_id) => {
            if validated.raw().root == ProjectRoot::Scene(scene_id) {
                return Err(ProjectWriteError::Invalid {
                    code: ProjectWriteErrorCode::ProtectedRootScene,
                });
            }
            let scene = next_manifest
                .scenes
                .iter_mut()
                .find(|scene| scene.id == scene_id)
                .ok_or(ProjectWriteError::Invalid {
                    code: ProjectWriteErrorCode::SceneNotFound,
                })?;
            scene.display_name = name.to_owned();
            placement_paths.push(crate::project::scene::authoring::scene_path(
                project_root,
                scene_id,
            ));
            placement_paths.extend(placement_paths_for_target(
                project_root,
                &validated,
                &target,
            )?);
        }
        ProjectWriteTarget::Model(model_id) => {
            let model = next_manifest
                .models
                .iter_mut()
                .find(|model| model.id == model_id)
                .ok_or(ProjectWriteError::Invalid {
                    code: ProjectWriteErrorCode::InvalidSelection,
                })?;
            model.display_name = name.to_owned();
            placement_paths.push(crate::project::model_wrapper::model_wrapper_path(
                project_root,
                model_id,
            ));
            placement_paths.extend(placement_paths_for_target(
                project_root,
                &validated,
                &target,
            )?);
        }
    }

    next_manifest
        .validate()
        .map_err(|_| ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::InvalidSelection,
        })?;
    placement_paths.push(crate::project::catalog::manifest_store::manifest_path(
        project_root,
    ));
    placement_paths.sort();
    placement_paths.dedup();

    service.stage_mutations.ensure_capacity(project_root)?;
    let transaction_directory = project_root
        .join(".usdhub")
        .join(".transactions")
        .join(format!("rename-{}", Uuid::new_v4()));
    fs::create_dir_all(&transaction_directory).map_err(|_| filesystem_error())?;
    let mut backups = Vec::new();
    let result = (|| {
        for (ordinal, original) in placement_paths.iter().enumerate() {
            let backup = transaction_directory
                .join("files")
                .join(format!("{ordinal}.backup"));
            if let Some(parent) = backup.parent() {
                fs::create_dir_all(parent).map_err(|_| filesystem_error())?;
            }
            fs::copy(original, &backup).map_err(|_| filesystem_error())?;
            backups.push(FileBackup {
                original: original.clone(),
                backup,
            });
        }

        match target {
            ProjectWriteTarget::Project(_) => {
                let root_scene_id = match next_manifest.root {
                    ProjectRoot::Scene(root_scene_id) => root_scene_id,
                    ProjectRoot::Empty | ProjectRoot::Model(_) => {
                        return Err(ProjectWriteError::Invalid {
                            code: ProjectWriteErrorCode::InvalidRootForComposition,
                        });
                    }
                };
                crate::project::scene::authoring::update_display_name_atomic(
                    &crate::project::scene::authoring::scene_path(project_root, root_scene_id),
                    "/SceneRoot",
                    name,
                )
                .map_err(|_| filesystem_error())?;
            }
            ProjectWriteTarget::Scene(scene_id) => {
                crate::project::scene::authoring::update_display_name_atomic(
                    &crate::project::scene::authoring::scene_path(project_root, scene_id),
                    "/SceneRoot",
                    name,
                )
                .map_err(|_| filesystem_error())?;
                update_placements(project_root, &validated, &target, name)?;
            }
            ProjectWriteTarget::Model(model_id) => {
                crate::project::scene::authoring::update_display_name_atomic(
                    &crate::project::model_wrapper::model_wrapper_path(project_root, model_id),
                    "/ModelRoot",
                    name,
                )
                .map_err(|_| filesystem_error())?;
                update_placements(project_root, &validated, &target, name)?;
            }
        }

        ManifestStore::write_manifest_atomic(project_root, &next_manifest)
            .map_err(|_| filesystem_error())?;
        Ok(())
    })();
    if let Err(error) = result {
        restore_file_backups(&backups);
        let _ = fs::remove_dir_all(&transaction_directory);
        return Err(error);
    }

    let project = match super::inspection::project_summary(&next_manifest, project_root) {
        Ok(project) => project,
        Err(error) => {
            restore_file_backups(&backups);
            let _ = fs::remove_dir_all(&transaction_directory);
            return Err(error);
        }
    };
    if let Err(error) = service.stage_mutations.submit_for_project(
        project_root,
        super::ProjectStageMutation::Rename {
            project_id,
            target: target.clone(),
            name: name.to_owned(),
        },
    ) {
        restore_file_backups(&backups);
        let _ = fs::remove_dir_all(&transaction_directory);
        return Err(error);
    }
    let cache_target = match &target {
        ProjectWriteTarget::Project(_) => crate::project::cache::ProjectCacheTarget::ProjectRoot,
        ProjectWriteTarget::Scene(id) => {
            crate::project::cache::ProjectCacheTarget::Scene { id: id.to_string() }
        }
        ProjectWriteTarget::Model(id) => {
            crate::project::cache::ProjectCacheTarget::Model { id: id.to_string() }
        }
    };
    let _ = service
        .cache_warm
        .enqueue_affected(project_root, cache_target);
    let _ = fs::remove_dir_all(&transaction_directory);
    Ok(ProjectRenameResponse { project, target })
}

fn update_placements(
    project_root: &std::path::Path,
    manifest: &usd_project::ValidatedProjectManifest,
    target: &ProjectWriteTarget,
    name: &str,
) -> Result<(), ProjectWriteError> {
    #[cfg(test)]
    let mut rewritten = 0;
    for scene in manifest.scenes() {
        let scene_path = crate::project::scene::authoring::scene_path(project_root, scene.id);
        let members = crate::project::scene::authoring::read_scene_members(&scene_path, scene.id)
            .map_err(|_| filesystem_error())?;
        for member in members {
            let matches = match (target, member.target) {
                (ProjectWriteTarget::Scene(id), SceneMemberTarget::Scene(target_id)) => {
                    *id == target_id
                }
                (ProjectWriteTarget::Model(id), SceneMemberTarget::Model(target_id)) => {
                    *id == target_id
                }
                _ => false,
            };
            if matches {
                #[cfg(test)]
                {
                    if should_fail_after_placement(rewritten) {
                        return Err(filesystem_error());
                    }
                }
                crate::project::scene::authoring::update_member_display_name_atomic(
                    &scene_path,
                    scene.id,
                    member.id,
                    name,
                )
                .map_err(|_| filesystem_error())?;
                #[cfg(test)]
                {
                    rewritten += 1;
                }
            }
        }
    }
    Ok(())
}

fn placement_paths_for_target(
    project_root: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
    target: &ProjectWriteTarget,
) -> Result<Vec<PathBuf>, ProjectWriteError> {
    let mut paths = Vec::new();
    for scene in manifest.scenes() {
        let scene_path = crate::project::scene::authoring::scene_path(project_root, scene.id);
        let members = crate::project::scene::authoring::read_scene_members(&scene_path, scene.id)
            .map_err(|_| filesystem_error())?;
        if members
            .iter()
            .any(|member| match (target, member.target.clone()) {
                (ProjectWriteTarget::Scene(id), SceneMemberTarget::Scene(target_id)) => {
                    *id == target_id
                }
                (ProjectWriteTarget::Model(id), SceneMemberTarget::Model(target_id)) => {
                    *id == target_id
                }
                _ => false,
            })
        {
            paths.push(scene_path);
        }
    }
    Ok(paths)
}

struct FileBackup {
    original: PathBuf,
    backup: PathBuf,
}

fn restore_file_backups(backups: &[FileBackup]) {
    for backup in backups.iter().rev() {
        let _ = fs::remove_file(&backup.original);
        let _ = fs::rename(&backup.backup, &backup.original);
    }
}

fn filesystem_error() -> ProjectWriteError {
    ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::FilesystemFailure,
    }
}

#[cfg(test)]
#[path = "rename_tests.rs"]
mod tests;
