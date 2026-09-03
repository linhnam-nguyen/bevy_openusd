//! Clone/import coverage for application-local Project registration identity.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use project_protocol::{ProjectReadCommand, ProjectReadRequest, ProjectReadResponse};
use usd_project::{ProjectContentNode, ProjectId};

use crate::project::{catalog::manifest_store::ManifestStore, service::ProjectApplicationService};

use super::{artifacts, fixture};

#[derive(Debug)]
struct Trace {
    seed: u64,
    project_root: PathBuf,
    fixture_ids: Vec<usd_project::SceneId>,
    operations: Vec<String>,
}

impl Trace {
    fn failure(&self, message: impl std::fmt::Display) -> String {
        format!(
            "{message}; seed={:#018X}; fixture_ids={:?}; project={}; operations={:?}",
            self.seed,
            self.fixture_ids,
            self.project_root.display(),
            self.operations
        )
    }
}

pub(super) fn run_seed(seed: u64) -> Result<(), String> {
    let run_directory = artifacts::clean_run_directory(&format!("c7-{seed:016x}"))?;
    let clone_directory = artifacts::clean_output_directory("clones", &format!("c7-{seed:016x}"))?;
    let projects_root = run_directory.join("projects");
    fs::create_dir(&projects_root).map_err(|error| format!("create C7 projects root: {error}"))?;
    let project_root = projects_root.join("Proj_T");
    let mut service = ProjectApplicationService::open(run_directory.join("workspace.json"))
        .map_err(|error| format!("open C7 service: {error}"))?;
    let fixture = fixture::create(&mut service, &projects_root)
        .map_err(|error| format!("create C7 fixture: {error}"))?;
    let fixture_ids = fixture.scenes.iter().map(|scene| scene.id).collect();
    let mut trace = Trace {
        seed,
        project_root: project_root.clone(),
        fixture_ids,
        operations: Vec::new(),
    };
    let clone_root = clone_directory.join("Proj_T_clone");
    trace.operations.push(format!(
        "clone {} -> {}",
        project_root.display(),
        clone_root.display()
    ));
    copy_tree(&project_root, &clone_root)
        .map_err(|error| trace.failure(format!("clone Project repository: {error}")))?;
    let source_content_before = tracked_content(&project_root)
        .map_err(|error| trace.failure(format!("snapshot source content: {error}")))?;
    let clone_content_before = tracked_content(&clone_root)
        .map_err(|error| trace.failure(format!("snapshot clone content: {error}")))?;
    if source_content_before != clone_content_before {
        return Err(trace.failure("filesystem clone changed initial Project content"));
    }

    trace
        .operations
        .push("normal inspect/import clone".to_owned());
    let inspection = service
        .inspect_project(&clone_root)
        .map_err(|error| trace.failure(format!("inspect cloned Project: {error}")))?;
    let imported = service
        .import_project(&clone_root, &inspection)
        .map_err(|error| trace.failure(format!("normal cloned Project import: {error}")))?;
    let source_app_id = fixture.project.id;
    let clone_app_id = imported.id;
    if source_app_id == clone_app_id {
        return Err(trace.failure("clone import reused the source AppProjectId"));
    }
    let source_entry = service
        .registry
        .get(source_app_id)
        .ok_or_else(|| trace.failure("source AppProjectId was displaced"))?;
    let clone_entry = service
        .registry
        .get(clone_app_id)
        .ok_or_else(|| trace.failure("clone AppProjectId was not registered"))?;
    if source_entry.content_project_id() != clone_entry.content_project_id()
        || source_entry.content_project_id() != fixture.project.id
    {
        return Err(trace.failure("cloned content identity did not remain equal"));
    }
    if source_entry.repository_locator() == clone_entry.repository_locator() {
        return Err(trace.failure("source and clone registry locations are equal"));
    }
    let source_scene_ids = read_scene_ids(&service, source_app_id, &trace)?;
    let clone_scene_ids = read_scene_ids(&service, clone_app_id, &trace)?;
    if source_scene_ids != clone_scene_ids {
        return Err(trace.failure("clone Scene identities differ from source content"));
    }
    if read_tree_project_id(&service, clone_app_id, &trace)? != clone_app_id {
        return Err(trace.failure("clone ProjectTree is not scoped to clone AppProjectId"));
    }

    let source_content_after = tracked_content(&project_root)
        .map_err(|error| trace.failure(format!("snapshot source after import: {error}")))?;
    let clone_content_after = tracked_content(&clone_root)
        .map_err(|error| trace.failure(format!("snapshot clone after import: {error}")))?;
    if source_content_after != source_content_before || clone_content_after != clone_content_before
    {
        return Err(trace.failure("registration/import changed tracked Project content"));
    }
    let manifest = ManifestStore::read_validated(&clone_root)
        .map_err(|error| trace.failure(format!("read cloned manifest: {error}")))?;
    if manifest.raw().project_id != fixture.project.id {
        return Err(trace.failure("clone manifest content ProjectId changed"));
    }
    Ok(())
}

fn read_scene_ids(
    service: &ProjectApplicationService,
    project_id: ProjectId,
    trace: &Trace,
) -> Result<Vec<usd_project::SceneId>, String> {
    let response = service
        .execute(ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(
            project_id,
        )))
        .result
        .map_err(|error| trace.failure(format!("read ProjectTree: {error}")))?;
    let ProjectReadResponse::ProjectTree { nodes, .. } = response else {
        return Err(trace.failure("ProjectTree query returned another response"));
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
    service: &ProjectApplicationService,
    project_id: ProjectId,
    trace: &Trace,
) -> Result<ProjectId, String> {
    let response = service
        .execute(ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(
            project_id,
        )))
        .result
        .map_err(|error| trace.failure(format!("read clone ProjectTree: {error}")))?;
    let ProjectReadResponse::ProjectTree { project_id, .. } = response else {
        return Err(trace.failure("clone ProjectTree query returned another response"));
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
