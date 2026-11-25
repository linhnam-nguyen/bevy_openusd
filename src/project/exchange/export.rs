//! Dependency-complete USDZ export for canonical Project Scenes.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use openusd::{
    ar::{join_package_relative_path, split_package_relative_path_outer},
    usd::{PrimPredicate, Stage},
    usdz::ArchiveWriter,
};
use project_protocol::{
    ProjectExportSceneRequest, ProjectReadError, ProjectSceneExportResponse, ProjectWriteError,
    ProjectWriteErrorCode,
};
use usd_project::SceneId;
use uuid::Uuid;

use crate::project::{
    assets::closure::{dependency_containment_report, scene_service::scene_dependency_closure},
    models::wrapper::model_wrapper_path,
    service::ProjectApplicationService,
    storage::ProjectStorageLayout,
};
use crate::history::commit::runtime::RuntimeLeaseGuard;

const SCENES_DIRECTORY: &str = "scenes";
const MODELS_DIRECTORY: &str = "models";
const SCENE_SOURCES_DIRECTORY: &str = "imports/scenes";

struct ExportEntry {
    source: PathBuf,
    archive: String,
}

pub(crate) fn export_scene(
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
    let runtime_authority = service.publication_coordinator.runtime_authority_arc();
    let runtime_snapshot =
        runtime_authority.snapshot_for_export(&project_root, project_id, request.scene_id)?;
    let _runtime_lease = RuntimeLeaseGuard::new(
        runtime_authority,
        project_root.to_owned(),
        project_id,
        runtime_snapshot.as_ref(),
    );
    let parent = destination.parent().ok_or(ProjectWriteError::Invalid {
        code: ProjectWriteErrorCode::ExportDestinationInvalid,
    })?;
    let temporary = parent.join(format!(".{file_name}.{}.usdz", Uuid::new_v4()));
    let result: std::result::Result<(), ProjectWriteError> = (|| {
        if let Some(snapshot) = runtime_snapshot.as_ref() {
            let live_root = tempfile::tempdir().map_err(|_| export_error())?;
            let live_root_path = live_root.path().join("active-root.usda");
            fs::write(&live_root_path, &snapshot.root_layer).map_err(|_| export_error())?;
            write_archive_with_root_source(
                project_root,
                &manifest,
                request.scene_id,
                &temporary,
                Some((&live_root_path, snapshot.scene_id)),
            )?;
        } else {
            write_archive(project_root, &manifest, request.scene_id, &temporary)?;
        }
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

pub(super) fn write_archive_with_root_source(
    project_root: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
    root_scene: SceneId,
    destination: &Path,
    runtime_override: Option<(&Path, SceneId)>,
) -> Result<(), ProjectWriteError> {
    let (scenes, models) = scene_dependency_closure(project_root, manifest.raw(), root_scene)
        .map_err(|_| export_error())?;
    let mut entries = export_entries(project_root, manifest, &scenes, &models)?;
    let root_source = scene_path(project_root, root_scene);
    if !entries.iter().any(|entry| entry.source == root_source) {
        return Err(export_error());
    }
    let mapping = complete_export_mapping(project_root, &root_source, &mut entries)?;
    let root_read_source = runtime_override
        .filter(|(_, scene_id)| *scene_id == root_scene)
        .map(|(source, _)| source)
        .unwrap_or(&root_source);
    let mut archive = ArchiveWriter::create(destination).map_err(|_| export_error())?;
    let root_bytes = read_export_bytes(root_read_source, &root_source, "scene.usda", &mapping)?;
    archive
        .add_layer("scene.usda", &root_bytes)
        .map_err(|_| export_error())?;

    let mut ordered = entries;
    ordered.sort_by(|left, right| left.archive.cmp(&right.archive));
    for entry in ordered {
        let source = runtime_override
            .filter(|(_, scene_id)| entry.source == scene_path(project_root, *scene_id))
            .map(|(source, _)| source)
            .unwrap_or(&entry.source);
        let bytes = read_export_bytes(source, &entry.source, &entry.archive, &mapping)?;
        archive
            .add_layer(&entry.archive, &bytes)
            .map_err(|_| export_error())?;
    }
    archive.finish().map_err(|_| export_error())?;
    Ok(())
}

fn complete_export_mapping(
    project_root: &Path,
    root_source: &Path,
    entries: &mut Vec<ExportEntry>,
) -> Result<HashMap<PathBuf, String>, ProjectWriteError> {
    let report = dependency_containment_report(project_root, root_source).map_err(|error| {
        log::warn!("Cannot package Project Scene dependencies: {error:#}");
        export_error()
    })?;
    let project_root = project_root.canonicalize().map_err(|_| export_error())?;
    let mut mapping = HashMap::new();
    for entry in entries.iter() {
        let canonical = entry.source.canonicalize().map_err(|_| export_error())?;
        if !canonical.starts_with(&project_root)
            || mapping.insert(canonical, entry.archive.clone()).is_some()
        {
            return Err(export_error());
        }
    }
    for source in report.layers.into_iter().chain(report.non_layer_assets) {
        if mapping.contains_key(&source) {
            continue;
        }
        let relative = source
            .strip_prefix(&project_root)
            .map_err(|_| export_error())?
            .to_str()
            .ok_or_else(export_error)?
            .replace('\\', "/");
        let archive = format!("dependencies/{relative}");
        mapping.insert(source.clone(), archive.clone());
        entries.push(ExportEntry { source, archive });
    }
    Ok(mapping)
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
        let source_directory =
            ProjectStorageLayout::new(project_root).readable_scene_import_dir(scene_id);
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
        let wrapper = model_wrapper_path(project_root, model_id);
        entries.push(ExportEntry {
            source: wrapper,
            archive: format!("{MODELS_DIRECTORY}/{model_id}/model.usda"),
        });
        let source_directory =
            ProjectStorageLayout::new(project_root).readable_model_import_dir(model_id);
        add_directory_entries(
            &source_directory,
            &format!("imports/models/{model_id}"),
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
    asset_anchor: &Path,
    archive_path: &str,
    mapping: &HashMap<PathBuf, String>,
) -> Result<Vec<u8>, ProjectWriteError> {
    let metadata = fs::symlink_metadata(source).map_err(|_| export_error())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(export_error());
    }
    let ext = source
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());
    let is_usd_layer = matches!(ext.as_deref(), Some("usd" | "usda" | "usdc"));
    let bytes = fs::read(source).map_err(|_| export_error())?;
    if !is_usd_layer {
        return Ok(bytes);
    }
    rewrite_layer_asset_paths(source, asset_anchor, &bytes, archive_path, mapping)
}

fn rewrite_layer_asset_paths(
    source: &Path,
    asset_anchor: &Path,
    bytes: &[u8],
    current_archive_path: &str,
    mapping: &HashMap<PathBuf, String>,
) -> Result<Vec<u8>, ProjectWriteError> {
    let ext = source
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("usda");
    let tag = format!("export.{ext}");
    let layer =
        openusd::sdf::Layer::from_bytes(&tag, bytes.to_vec()).map_err(|_| export_error())?;
    let handle = openusd::sdf::LayerRegistry::default()
        .adopt(layer)
        .map_err(|_| export_error())?;
    openusd::usd_utils::modify_asset_paths(
        &handle,
        |asset| {
            if asset.is_empty() {
                return String::new();
            }
            let packaged = split_package_relative_path_outer(asset);
            let asset_path = Path::new(packaged.as_ref().map_or(asset, |(package, _)| package));
            let resolved = if asset_path.is_absolute() {
                asset_path.to_owned()
            } else {
                asset_anchor
                    .parent()
                    .map(|parent| parent.join(asset_path))
                    .unwrap_or_else(|| asset_path.to_owned())
            };
            if let Some(target) = resolved
                .canonicalize()
                .ok()
                .and_then(|path| mapping.get(&path))
            {
                let relative = relative_archive_path(current_archive_path, target);
                if let Some((_, member)) = packaged {
                    join_package_relative_path(&relative, &member)
                } else {
                    relative
                }
            } else {
                asset.to_owned()
            }
        },
        true,
    )
    .map_err(|_| export_error())?;

    if ext.eq_ignore_ascii_case("usdc") {
        let temp = tempfile::tempdir().map_err(|_| export_error())?;
        let out_path = temp.path().join("rewritten.usdc");
        let out_str = out_path.to_str().ok_or_else(export_error)?;
        handle.export(out_str).map_err(|_| export_error())?;
        fs::read(&out_path).map_err(|_| export_error())
    } else {
        handle
            .export_to_string()
            .map(String::into_bytes)
            .map_err(|_| export_error())
    }
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
    let archive = path.canonicalize().map_err(|_| export_error())?;
    let stage =
        Stage::open(archive.to_str().ok_or_else(export_error)?).map_err(|_| export_error())?;
    stage
        .traverse(PrimPredicate::ALL, |_| {})
        .map_err(|_| export_error())?;
    if !stage.composition_errors().is_empty() {
        return Err(export_error());
    }
    for identifier in stage.layer_identifiers() {
        let containing_archive =
            split_package_relative_path_outer(&identifier).map_or(identifier, |(outer, _)| outer);
        if Path::new(&containing_archive).canonicalize().ok().as_ref() != Some(&archive) {
            return Err(export_error());
        }
    }
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

pub(super) fn export_error() -> ProjectWriteError {
    ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::ExportFailed,
    }
}
