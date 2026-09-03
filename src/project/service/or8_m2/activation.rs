//! Three scoped Scene activations through the authoritative service.

use std::{fs, path::PathBuf};

use project_protocol::{
    ProjectActivationCommand, ProjectReadCommand, ProjectReadRequest, ProjectReadResponse,
    ProjectStageTarget,
};
use usd_project::SceneId;

use crate::project::{
    catalog::manifest_store::ManifestStore, service::ProjectApplicationService,
    storage::ProjectStorageLayout,
};

use super::{artifacts, fixture, rng::DeterministicRng};

#[derive(Debug)]
struct Trace {
    seed: u64,
    project_root: PathBuf,
    fixture_ids: Vec<SceneId>,
    decisions: Vec<String>,
    operations: Vec<String>,
}

impl Trace {
    fn new(seed: u64, project_root: PathBuf, fixture_ids: Vec<SceneId>) -> Self {
        Self {
            seed,
            project_root,
            fixture_ids,
            decisions: Vec::new(),
            operations: Vec::new(),
        }
    }

    fn failure(&self, message: impl std::fmt::Display) -> String {
        format!(
            "{message}; seed={:#018X}; fixture_ids={:?}; project={}; decisions={:?}; operations={:?}",
            self.seed,
            self.fixture_ids,
            self.project_root.display(),
            self.decisions,
            self.operations
        )
    }
}

pub(super) fn run_seed(seed: u64) -> Result<(), String> {
    let run_directory = artifacts::clean_run_directory(&format!("c4-{seed:016x}"))?;
    let projects_root = run_directory.join("projects");
    fs::create_dir(&projects_root).map_err(|error| format!("create C4 projects root: {error}"))?;
    let project_root = projects_root.join("Proj_T");
    let mut service = ProjectApplicationService::open(run_directory.join("workspace.json"))
        .map_err(|error| format!("open C4 service: {error}"))?;
    let fixture = fixture::create(&mut service, &projects_root)
        .map_err(|error| format!("create C4 fixture: {error}"))?;
    let fixture_ids = fixture.scenes.iter().map(|scene| scene.id).collect();
    let mut trace = Trace::new(seed, project_root.clone(), fixture_ids);
    let mut rng = DeterministicRng::seeded(seed);
    let mut eligible = [
        fixture.identity("Sc1").id,
        fixture.identity("Sc1.2.3").id,
        fixture.identity("Sc2.1").id,
    ];
    for index in (1..eligible.len()).rev() {
        let swap = rng.choose_index(index + 1);
        trace
            .decisions
            .push(format!("activation_order[{index}] swap={swap}"));
        eligible.swap(index, swap);
    }

    let manifest_before = read_manifest_bytes(&project_root)?;
    let tree_before = read_tree(&service, fixture.project.id)?;
    let expected_latest_scene = eligible[eligible.len() - 1];
    let mut session = crate::project::service::ProjectStageActivationSession::default();
    let mut stale_completion = None;
    let mut previous_generation = 0;
    for (index, scene_id) in eligible.into_iter().enumerate() {
        let generation = index as u64 + 1;
        let request_id = format!("m2-c4-{seed:016x}-{index}");
        let command = ProjectActivationCommand::new(
            request_id.clone(),
            generation,
            fixture.project.id,
            ProjectStageTarget::Scene(scene_id),
        );
        command
            .validate()
            .map_err(|error| trace.failure(format!("invalid activation command: {error}")))?;
        if generation <= previous_generation {
            return Err(trace.failure("activation generation is not monotonic"));
        }
        trace.operations.push(format!(
            "activate scene={scene_id} request={request_id} generation={generation}"
        ));

        let target = service
            .resolve_stage_activation(fixture.project.id, command.target.clone())
            .map_err(|error| trace.failure(format!("activation resolution failed: {error}")))?
            .ok_or_else(|| trace.failure("eligible Scene resolved to no stage"))?;
        if target.target != command.target || target.project_id != fixture.project.id {
            return Err(trace.failure("authoritative activation target does not match command"));
        }
        if !session.observe_request("c4-session", &command) {
            return Err(trace.failure("activation request was not admitted"));
        }
        let target_snapshot = target.clone();
        let snapshot = session
            .complete("c4-session", &command, target)
            .map_err(|error| trace.failure(format!("activation completion: {error}")))?;
        if snapshot.project_id != fixture.project.id
            || snapshot.target != command.target
            || snapshot.generation != generation
            || snapshot.stage_path != target_snapshot.path
            || snapshot.hierarchy_paths.is_empty()
            || snapshot.bim_snapshot_id.is_empty()
            || !snapshot
                .bim_entity_paths
                .iter()
                .all(|path| snapshot.hierarchy_paths.iter().any(|item| item == path))
        {
            return Err(trace.failure("active Scene/session identity is incomplete"));
        }
        trace.operations.push(format!(
            "active snapshot generation={} stage={} bim_snapshot={} bim_entities={}",
            snapshot.generation,
            snapshot.stage_path.display(),
            snapshot.bim_snapshot_id,
            snapshot.bim_entity_paths.len()
        ));
        if index == 1 {
            stale_completion = Some((command, target_snapshot));
        }
        previous_generation = generation;

        if read_manifest_bytes(&project_root)? != manifest_before {
            return Err(trace.failure("Project manifest changed during Scene activation"));
        }
        if read_tree(&service, fixture.project.id)? != tree_before {
            return Err(trace.failure("ProjectTree changed during Scene activation"));
        }
        let manifest = ManifestStore::read_validated(&project_root)
            .map_err(|error| trace.failure(format!("read active Project manifest: {error}")))?;
        if manifest.scene(scene_id).is_none() {
            return Err(trace.failure("active Scene is absent from its Project scope"));
        }
    }
    let (stale_command, stale_target) =
        stale_completion.expect("three activations include a stale candidate");
    if session
        .complete("c4-session", &stale_command, stale_target)
        .is_ok()
    {
        return Err(trace.failure("stale Scene completion replaced the latest active identity"));
    }
    let active = session
        .active()
        .ok_or_else(|| trace.failure("active Scene/session snapshot is missing"))?;
    if active.generation != 3 || active.target != ProjectStageTarget::Scene(expected_latest_scene) {
        return Err(trace.failure("latest active Scene/session identity was not retained"));
    }
    Ok(())
}

fn read_manifest_bytes(project_root: &std::path::Path) -> Result<Vec<u8>, String> {
    fs::read(ProjectStorageLayout::new(project_root).readable_manifest_path())
        .map_err(|error| format!("read Project manifest snapshot: {error}"))
}

fn read_tree(
    service: &ProjectApplicationService,
    project_id: usd_project::ProjectId,
) -> Result<ProjectReadResponse, String> {
    service
        .execute(ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(
            project_id,
        )))
        .result
        .map_err(|error| format!("read ProjectTree: {error}"))
}
