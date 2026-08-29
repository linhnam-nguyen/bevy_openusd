//! Authoritative rename transactions for Project-managed content.

use project_protocol::{
    ProjectRenameResponse, ProjectWriteError, ProjectWriteErrorCode, ProjectWriteTarget,
};
use usd_project::{ProjectRoot, SceneMemberTarget};

use super::ProjectApplicationService;
use crate::project::catalog::manifest_store::ManifestStore;

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
            crate::project::scene::authoring::update_display_name_atomic(
                &crate::project::scene::authoring::scene_path(project_root, root_scene_id),
                "/SceneRoot",
                name,
            )
            .map_err(|_| filesystem_error())?;
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
            crate::project::scene::authoring::update_display_name_atomic(
                &crate::project::scene::authoring::scene_path(project_root, scene_id),
                "/SceneRoot",
                name,
            )
            .map_err(|_| filesystem_error())?;
            update_placements(service, project_root, &validated, &target, name)?;
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
            crate::project::scene::authoring::update_display_name_atomic(
                &crate::project::model_wrapper::model_wrapper_path(project_root, model_id),
                "/ModelRoot",
                name,
            )
            .map_err(|_| filesystem_error())?;
            update_placements(service, project_root, &validated, &target, name)?;
        }
    }

    next_manifest
        .validate()
        .map_err(|_| ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::InvalidSelection,
        })?;
    ManifestStore::write_manifest_atomic(project_root, &next_manifest)
        .map_err(|_| filesystem_error())?;
    service.stage_mutations.submit_for_project(
        project_root,
        super::ProjectStageMutation::Rename {
            project_id,
            target: target.clone(),
            name: name.to_owned(),
        },
    )?;
    let _ = service.cache_warm.enqueue_affected(
        project_root,
        crate::project::cache::ProjectCacheTarget::ProjectRoot,
    );
    let project = super::inspection::project_summary(&next_manifest, project_root)?;
    Ok(ProjectRenameResponse { project, target })
}

fn update_placements(
    _service: &ProjectApplicationService,
    project_root: &std::path::Path,
    manifest: &usd_project::ValidatedProjectManifest,
    target: &ProjectWriteTarget,
    name: &str,
) -> Result<(), ProjectWriteError> {
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
                crate::project::scene::authoring::update_member_display_name_atomic(
                    &scene_path,
                    scene.id,
                    member.id,
                    name,
                )
                .map_err(|_| filesystem_error())?;
            }
        }
    }
    Ok(())
}

fn filesystem_error() -> ProjectWriteError {
    ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::FilesystemFailure,
    }
}

#[cfg(test)]
#[path = "rename_tests.rs"]
mod tests;
