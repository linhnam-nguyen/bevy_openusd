//! Export/reimport and cloned Project validation for the OR8 M2 matrix.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use openusd::usd::{InitialLoadSet, PrimPredicate, Stage};
use project_protocol::{
    LocalSelectionToken, PlacementSpec, ProjectExportSceneRequest, ProjectReadCommand,
    ProjectReadRequest, ProjectReadResponse, ProjectWriteTarget,
};
use usd_project::{ProjectContentNode, SceneId, SceneMemberTarget};

use crate::project::{
    catalog::manifest_store::ManifestStore,
    scene::{
        authoring::{read_scene_members, scene_path},
        inspection::inspect_composition,
    },
};

use super::matrix::Context;

pub(super) fn export_roundtrip(context: &mut Context) -> Result<(), String> {
    let (source_scene, target_scene) = select_roundtrip_scenes(context)?;
    let destination = context.export_directory.join("matrix.usdz");
    context.trace.operation(format!(
        "export scene={source_scene} path={}",
        destination.display()
    ));
    context
        .service
        .export_scene(
            ProjectExportSceneRequest {
                project_id: context.fixture.project.id,
                scene_id: source_scene,
                destination: LocalSelectionToken::new("m2-c8-export"),
            },
            &destination,
        )
        .map_err(|error| context.trace.failure(format!("export Scene: {error}")))?;
    verify_export(&destination).map_err(|error| context.trace.failure(error))?;
    let inspection = inspect_composition(&destination).map_err(|error| {
        context
            .trace
            .failure(format!("inspect exported Scene: {error}"))
    })?;
    context.trace.operation(format!(
        "reimport path={} target={} mode=normal-adopt",
        destination.display(),
        target_scene
    ));
    let imported = context
        .service
        .adopt_scene(
            context.fixture.project.id,
            ProjectWriteTarget::Scene(target_scene),
            &destination,
            &inspection,
            "C8_Roundtrip".to_owned(),
            "m2-c8-reimport".to_owned(),
            10,
            PlacementSpec::Default,
        )
        .map_err(|error| context.trace.failure(format!("reimport Scene: {error}")))?;
    if imported.scene_id == source_scene {
        return Err(context
            .trace
            .failure("reimport reused exported Scene identity"));
    }
    let members = read_scene_members(
        &scene_path(&context.project_root, target_scene),
        target_scene,
    )
    .map_err(|error| {
        context
            .trace
            .failure(format!("read roundtrip parent: {error}"))
    })?;
    if !members
        .iter()
        .any(|member| member.target == SceneMemberTarget::Scene(imported.scene_id))
    {
        return Err(context.trace.failure("reimported Scene is not placed"));
    }
    let manifest = ManifestStore::read_validated(&context.project_root).map_err(|error| {
        context
            .trace
            .failure(format!("read roundtrip manifest: {error}"))
    })?;
    if manifest.scene(imported.scene_id).is_none() {
        return Err(context
            .trace
            .failure("reimported Scene is absent from manifest"));
    }
    Ok(())
}

fn select_roundtrip_scenes(context: &mut Context) -> Result<(SceneId, SceneId), String> {
    let current_scene_ids = read_scene_ids(&context.service, context.fixture.project.id)?;
    let mut eligible = context
        .fixture
        .scenes
        .iter()
        .filter(|scene| {
            scene
                .parent
                .is_some_and(|parent| parent != context.fixture.root_scene_id)
        })
        .filter(|scene| current_scene_ids.contains(&scene.id))
        .map(|scene| scene.id)
        .collect::<Vec<_>>();
    if eligible.len() < 2 {
        return Err(context
            .trace
            .failure("fewer than two surviving nested canonical Scenes are export eligible"));
    }
    for index in (1..eligible.len()).rev() {
        eligible.swap(index, context.rng.choose_index(index + 1));
    }
    let source_scene = eligible[0];
    let target_scene = eligible[1];
    context.trace.decision(format!(
        "roundtrip_selection surviving_nested source={source_scene} target={target_scene} candidates={eligible:?}"
    ));
    Ok((source_scene, target_scene))
}

pub(super) fn clone_and_validate(context: &mut Context) -> Result<(), String> {
    let clone_root = context.clone_directory.join("Proj_T_clone");
    context.trace.operation(format!(
        "clone {} -> {}",
        context.project_root.display(),
        clone_root.display()
    ));
    copy_tree(&context.project_root, &clone_root)
        .map_err(|error| context.trace.failure(format!("clone Project: {error}")))?;
    let before_source = tracked_content(&context.project_root)
        .map_err(|error| context.trace.failure(format!("snapshot source: {error}")))?;
    let before_clone = tracked_content(&clone_root)
        .map_err(|error| context.trace.failure(format!("snapshot clone: {error}")))?;
    if before_source != before_clone {
        return Err(context.trace.failure("clone content differs before import"));
    }
    let inspection = context
        .service
        .inspect_project(&clone_root)
        .map_err(|error| context.trace.failure(format!("inspect clone: {error}")))?;
    let imported = context
        .service
        .import_project(&clone_root, &inspection)
        .map_err(|error| context.trace.failure(format!("import clone: {error}")))?;
    let source_app_id = context.fixture.project.id;
    if imported.id == source_app_id {
        return Err(context.trace.failure("clone reused source AppProjectId"));
    }
    let source_entry = context
        .service
        .registry
        .get(source_app_id)
        .ok_or_else(|| context.trace.failure("source registration disappeared"))?;
    let clone_entry = context
        .service
        .registry
        .get(imported.id)
        .ok_or_else(|| context.trace.failure("clone registration missing"))?;
    if source_entry.content_project_id() != clone_entry.content_project_id()
        || source_entry.content_project_id()
            != ManifestStore::read_validated(&context.project_root)
                .map_err(|error| {
                    context
                        .trace
                        .failure(format!("read source identity: {error}"))
                })?
                .raw()
                .project_id
    {
        return Err(context.trace.failure("clone content identity changed"));
    }
    if source_entry.repository_locator() == clone_entry.repository_locator() {
        return Err(context
            .trace
            .failure("clone repository scope was not distinct"));
    }
    if read_scene_ids(&context.service, source_app_id)?
        != read_scene_ids(&context.service, imported.id)?
    {
        return Err(context
            .trace
            .failure("clone Scene identities differ from source"));
    }
    if read_tree_project_id(&context.service, imported.id)? != imported.id {
        return Err(context
            .trace
            .failure("clone ProjectTree has wrong application scope"));
    }
    if tracked_content(&context.project_root).map_err(|error| {
        context
            .trace
            .failure(format!("snapshot source after import: {error}"))
    })? != before_source
        || tracked_content(&clone_root).map_err(|error| {
            context
                .trace
                .failure(format!("snapshot clone after import: {error}"))
        })? != before_clone
    {
        return Err(context
            .trace
            .failure("clone import changed tracked content"));
    }
    context.trace.operation(format!(
        "identity source_app={source_app_id} clone_app={} content={:?}",
        imported.id,
        source_entry.content_project_id()
    ));
    Ok(())
}

fn verify_export(path: &Path) -> Result<(), String> {
    let file = fs::File::open(path).map_err(|error| format!("open USDZ: {error}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| format!("read USDZ: {error}"))?;
    if archive.is_empty() {
        return Err("exported USDZ is empty".to_owned());
    }
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("read USDZ entry: {error}"))?;
        if entry.name().starts_with('/') || entry.name().split('/').any(|part| part == "..") {
            return Err(format!("unsafe USDZ entry {:?}", entry.name()));
        }
    }
    drop(archive);
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(path.to_string_lossy().as_ref())
        .map_err(|error| format!("open exported stage: {error}"))?;
    stage
        .traverse(PrimPredicate::DEFAULT, |_| {})
        .map_err(|error| format!("traverse exported stage: {error}"))?;
    if !stage.composition_errors().is_empty() {
        return Err(format!(
            "export composition errors: {:?}",
            stage.composition_errors()
        ));
    }
    Ok(())
}

fn read_scene_ids(
    service: &crate::project::service::ProjectApplicationService,
    project_id: usd_project::ProjectId,
) -> Result<Vec<SceneId>, String> {
    let response = service.execute(ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(
        project_id,
    )));
    let ProjectReadResponse::ProjectTree { nodes, .. } = response
        .result
        .map_err(|error| format!("read ProjectTree: {error}"))?
    else {
        return Err("ProjectTree returned unexpected response".to_owned());
    };
    let mut ids = nodes
        .into_iter()
        .filter_map(|node| match node {
            ProjectContentNode::Scene { scene_id, .. } => Some(scene_id),
            ProjectContentNode::Model { .. }
            | ProjectContentNode::ScenePlacement { .. }
            | ProjectContentNode::ModelPlacement { .. } => None,
        })
        .collect::<Vec<_>>();
    ids.sort();
    Ok(ids)
}

fn read_tree_project_id(
    service: &crate::project::service::ProjectApplicationService,
    project_id: usd_project::ProjectId,
) -> Result<usd_project::ProjectId, String> {
    let response = service.execute(ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(
        project_id,
    )));
    let ProjectReadResponse::ProjectTree { project_id, .. } = response
        .result
        .map_err(|error| format!("read clone ProjectTree: {error}"))?
    else {
        return Err("clone ProjectTree returned unexpected response".to_owned());
    };
    Ok(project_id)
}

fn tracked_content(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, String> {
    let mut files = BTreeMap::new();
    collect_content(root, root, &mut files)?;
    Ok(files)
}

fn collect_content(
    root: &Path,
    current: &Path,
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
) -> Result<(), String> {
    for entry in
        fs::read_dir(current).map_err(|error| format!("read {}: {error}", current.display()))?
    {
        let entry = entry.map_err(|error| format!("read Project entry: {error}"))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|error| format!("relative Project path: {error}"))?;
        if relative
            .components()
            .any(|component| component.as_os_str() == ".git" || component.as_os_str() == ".usdhub")
        {
            continue;
        }
        if entry
            .file_type()
            .map_err(|error| format!("read Project entry type: {error}"))?
            .is_dir()
        {
            collect_content(root, &path, files)?;
        } else {
            files.insert(
                relative.to_owned(),
                fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?,
            );
        }
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("create {}: {error}", destination.display()))?;
    for entry in
        fs::read_dir(source).map_err(|error| format!("read {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("read clone entry: {error}"))?;
        if entry.file_name() == ".usdhub" {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| format!("read clone entry type: {error}"))?
            .is_dir()
        {
            copy_tree(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path).map_err(|error| {
                format!(
                    "copy {} -> {}: {error}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }
    Ok(())
}
