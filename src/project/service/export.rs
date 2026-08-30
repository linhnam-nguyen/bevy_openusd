//! Dependency-complete USDZ export for canonical Project Scenes.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use openusd::{
    usd::{PrimPredicate, Stage},
    usdz::ArchiveWriter,
};
use project_protocol::{
    ProjectExportSceneRequest, ProjectReadError, ProjectSceneExportResponse, ProjectWriteError,
    ProjectWriteErrorCode,
};
use usd_project::{SceneId, SceneMemberTarget};
use uuid::Uuid;

use super::ProjectApplicationService;

#[path = "live_export.rs"]
mod live_export;
pub(crate) use live_export::write_live_stage_archive;

const SCENES_DIRECTORY: &str = "scenes";
const MODELS_DIRECTORY: &str = "models";
const SCENE_SOURCES_DIRECTORY: &str = "imports/scenes";

struct ExportEntry {
    source: PathBuf,
    archive: String,
}

pub(super) fn export_scene(
    service: &mut ProjectApplicationService,
    request: ProjectExportSceneRequest,
    destination: &Path,
) -> Result<ProjectSceneExportResponse, ProjectWriteError> {
    let project_id = request.project_id;
    let (entry, manifest) = service
        .validated_project(project_id)
        .map_err(project_error)?;
    if manifest.scene(request.scene_id).is_none() {
        return Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::SceneNotFound,
        });
    }
    let file_name = validate_destination(destination)?;
    let publisher = service.publication_coordinator.publisher(project_id);
    let _guard = publisher.lock().map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::Busy,
    })?;

    let project_root = entry.repository_locator();
    let parent = destination.parent().ok_or(ProjectWriteError::Invalid {
        code: ProjectWriteErrorCode::ExportDestinationInvalid,
    })?;
    let temporary = parent.join(format!(".{file_name}.{}.usdz", Uuid::new_v4()));
    let result: std::result::Result<(), ProjectWriteError> = (|| {
        write_archive(project_root, &manifest, request.scene_id, &temporary)?;
        validate_archive(&temporary)?;
        fs::rename(&temporary, destination).map_err(|_| export_error())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;

    Ok(ProjectSceneExportResponse {
        project_id,
        scene_id: request.scene_id,
        file_name,
    })
}

fn validate_destination(destination: &Path) -> Result<String, ProjectWriteError> {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::ExportDestinationInvalid,
        })?;
    if !destination
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("usdz"))
        || destination.exists() && destination.is_dir()
    {
        return Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::ExportDestinationInvalid,
        });
    }
    if !destination.parent().is_some_and(|parent| parent.is_dir()) {
        return Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::ExportDestinationInvalid,
        });
    }
    Ok(file_name.to_owned())
}

fn write_archive(
    project_root: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
    root_scene: SceneId,
    destination: &Path,
) -> Result<(), ProjectWriteError> {
    write_archive_with_root_source(project_root, manifest, root_scene, destination, None)
}

fn write_archive_with_root_source(
    project_root: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
    root_scene: SceneId,
    destination: &Path,
    root_source_override: Option<&Path>,
) -> Result<(), ProjectWriteError> {
    let (scenes, models) = dependency_closure(project_root, manifest, root_scene)?;
    let entries = export_entries(project_root, manifest, &scenes, &models)?;
    let root_source = scene_path(project_root, root_scene);
    if !entries.iter().any(|entry| entry.source == root_source) {
        return Err(export_error());
    }
    let root_read_source = root_source_override.unwrap_or(&root_source);
    let mut archive = ArchiveWriter::create(destination).map_err(|_| export_error())?;
    let mapping = entries
        .iter()
        .map(|entry| (entry.source.clone(), entry.archive.clone()))
        .collect::<HashMap<_, _>>();
    let root_bytes = read_export_bytes(root_read_source, "scene.usda", &mapping)?;
    archive
        .add_layer("scene.usda", &root_bytes)
        .map_err(|_| export_error())?;

    let mut ordered = entries;
    ordered.sort_by(|left, right| left.archive.cmp(&right.archive));
    for entry in ordered {
        let bytes = read_export_bytes(&entry.source, &entry.archive, &mapping)?;
        archive
            .add_layer(&entry.archive, &bytes)
            .map_err(|_| export_error())?;
    }
    archive.finish().map_err(|_| export_error())?;
    Ok(())
}

fn dependency_closure(
    project_root: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
    root_scene: SceneId,
) -> Result<(HashSet<SceneId>, HashSet<usd_project::ModelId>), ProjectWriteError> {
    let mut members = HashMap::new();
    for scene in manifest.scenes() {
        let path = scene_path(project_root, scene.id);
        let scene_members = crate::project::scene::authoring::read_scene_members(&path, scene.id)
            .map_err(|_| export_error())?;
        members.insert(scene.id, scene_members);
    }

    let mut scenes = HashSet::from([root_scene]);
    let mut models = HashSet::new();
    let mut pending = vec![root_scene];
    while let Some(scene_id) = pending.pop() {
        for member in members.get(&scene_id).into_iter().flatten() {
            match member.target {
                SceneMemberTarget::Scene(child) if scenes.insert(child) => pending.push(child),
                SceneMemberTarget::Model(model_id) => {
                    models.insert(model_id);
                }
                SceneMemberTarget::Scene(_) => {}
            }
        }
    }
    for model_id in &models {
        if manifest.model(*model_id).is_none() {
            return Err(export_error());
        }
    }
    Ok((scenes, models))
}

fn export_entries(
    project_root: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
    scenes: &HashSet<SceneId>,
    models: &HashSet<usd_project::ModelId>,
) -> Result<Vec<ExportEntry>, ProjectWriteError> {
    let mut entries = Vec::new();
    let mut ordered_scenes = scenes.iter().copied().collect::<Vec<_>>();
    ordered_scenes.sort();
    for scene_id in ordered_scenes {
        if manifest.scene(scene_id).is_none() {
            return Err(export_error());
        }
        entries.push(ExportEntry {
            source: scene_path(project_root, scene_id),
            archive: format!("{SCENES_DIRECTORY}/{scene_id}.usda"),
        });
        let source_directory = project_root
            .join(".usdhub")
            .join(SCENE_SOURCES_DIRECTORY)
            .join(scene_id.to_string());
        add_directory_entries(
            &source_directory,
            &format!("{SCENE_SOURCES_DIRECTORY}/{scene_id}"),
            &mut entries,
        )?;
    }
    let mut ordered_models = models.iter().copied().collect::<Vec<_>>();
    ordered_models.sort();
    for model_id in ordered_models {
        if manifest.model(model_id).is_none() {
            return Err(export_error());
        }
        let model_directory = project_root
            .join(".usdhub")
            .join(MODELS_DIRECTORY)
            .join(model_id.to_string());
        let wrapper = model_directory.join("model.usda");
        entries.push(ExportEntry {
            source: wrapper,
            archive: format!("{MODELS_DIRECTORY}/{model_id}/model.usda"),
        });
        add_directory_entries(
            &model_directory.join("source"),
            &format!("{MODELS_DIRECTORY}/{model_id}/source"),
            &mut entries,
        )?;
    }
    Ok(entries)
}

fn add_directory_entries(
    source_directory: &Path,
    archive_prefix: &str,
    entries: &mut Vec<ExportEntry>,
) -> Result<(), ProjectWriteError> {
    if !source_directory.exists() {
        return Ok(());
    }
    let mut paths = Vec::new();
    collect_files(source_directory, &mut paths)?;
    for source in paths {
        let relative = source
            .strip_prefix(source_directory)
            .map_err(|_| export_error())?
            .to_string_lossy()
            .replace('\\', "/");
        entries.push(ExportEntry {
            source,
            archive: format!("{archive_prefix}/{relative}"),
        });
    }
    Ok(())
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), ProjectWriteError> {
    let mut children = fs::read_dir(directory)
        .map_err(|_| export_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| export_error())?;
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|_| export_error())?;
        if metadata.file_type().is_symlink() {
            return Err(export_error());
        }
        if metadata.is_dir() {
            collect_files(&path, files)?;
        } else if metadata.is_file() {
            files.push(path);
        } else {
            return Err(export_error());
        }
    }
    Ok(())
}

fn read_export_bytes(
    source: &Path,
    archive_path: &str,
    mapping: &HashMap<PathBuf, String>,
) -> Result<Vec<u8>, ProjectWriteError> {
    let metadata = fs::symlink_metadata(source).map_err(|_| export_error())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(export_error());
    }
    let bytes = fs::read(source).map_err(|_| export_error())?;
    let is_usd_text = source
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "usd" | "usda"));
    if !is_usd_text {
        return Ok(bytes);
    }
    let text = String::from_utf8(bytes).map_err(|_| export_error())?;
    rewrite_asset_paths(&text, archive_path, mapping).map(|text| text.into_bytes())
}

fn rewrite_asset_paths(
    text: &str,
    current_archive_path: &str,
    mapping: &HashMap<PathBuf, String>,
) -> Result<String, ProjectWriteError> {
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(open_offset) = text[cursor..].find('@') {
        let open = cursor + open_offset;
        output.push_str(&text[cursor..open + 1]);
        let Some(close_offset) = text[open + 1..].find('@') else {
            return Err(export_error());
        };
        let close = open + 1 + close_offset;
        let asset = &text[open + 1..close];
        if asset.starts_with('/') {
            let asset_path = Path::new(asset);
            let target = mapping.get(asset_path).ok_or_else(export_error)?;
            let relative = relative_archive_path(current_archive_path, target);
            output.push_str(&relative);
        } else {
            output.push_str(asset);
        }
        output.push('@');
        cursor = close + 1;
    }
    output.push_str(&text[cursor..]);
    Ok(output)
}

fn relative_archive_path(current: &str, target: &str) -> String {
    let mut base = current.split('/').collect::<Vec<_>>();
    let _ = base.pop();
    let target = target.split('/').collect::<Vec<_>>();
    let mut common = 0;
    while common < base.len() && common < target.len() && base[common] == target[common] {
        common += 1;
    }
    let mut parts = vec![".."; base.len() - common];
    parts.extend_from_slice(&target[common..]);
    if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    }
}

fn validate_archive(path: &Path) -> Result<(), ProjectWriteError> {
    let path = path.to_string_lossy();
    let stage = Stage::open(path.as_ref()).map_err(|_| export_error())?;
    stage
        .traverse(PrimPredicate::DEFAULT, |_| {})
        .map_err(|_| export_error())?;
    Ok(())
}

fn scene_path(project_root: &Path, scene_id: SceneId) -> PathBuf {
    crate::project::scene::authoring::scene_path(project_root, scene_id)
}

fn project_error(error: ProjectReadError) -> ProjectWriteError {
    match error {
        ProjectReadError::NotFound { .. } => ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::ProjectNotFound,
        },
        _ => export_error(),
    }
}

fn export_error() -> ProjectWriteError {
    ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::ExportFailed,
    }
}
