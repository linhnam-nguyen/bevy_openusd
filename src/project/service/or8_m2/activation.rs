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
    fixture::seed_bim_metadata(&fixture)?;
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
    let mut production = crate::viewport::ProductionActivationWorld::new();
    production.replace_selection(viewport_protocol::SceneAnchor::active_session("/SceneRoot"));
    let mut stale_completion = None;
    let mut latest_target = None;
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
        if !production.admit("c4-session", &command) {
            return Err(trace.failure("activation request was not admitted"));
        }
        if index == 1 {
            stale_completion = Some((command.clone(), target.clone()));
        }
        latest_target = Some(target.clone());
        let reply = production.apply("c4-session", &command, Ok(Some(target.clone())));
        if !matches!(
            reply.result,
            project_protocol::ProjectActivationResult::Activated { .. }
        ) {
            return Err(trace.failure("production activation completion was rejected"));
        }
        production.update();
        let observation = production
            .observe(&target.path, generation)
            .map_err(|error| trace.failure(error))?;
        trace.operations.push(format!(
            "active Bevy generation={} stage={} semantic_snapshot={} bim_snapshot={} hierarchy_nodes={}",
            observation.generation,
            observation.stage_path.display(),
            observation.semantic_snapshot_id,
            observation.bim_snapshot_id,
            observation.hierarchy_nodes
        ));
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
    let (stale_command, stale_target) = stale_completion
        .ok_or_else(|| trace.failure("three activations include a stale candidate"))?;
    let latest_target = latest_target.ok_or_else(|| trace.failure("latest target is missing"))?;
    let before_stale = production
        .observe(&latest_target.path, 3)
        .map_err(|error| trace.failure(error))?;
    let reply = production.apply("c4-session", &stale_command, Ok(Some(stale_target)));
    if !matches!(
        reply.result,
        project_protocol::ProjectActivationResult::Failed { .. }
    ) {
        return Err(trace.failure("stale Scene completion was accepted"));
    }
    let after_stale = production
        .observe(&latest_target.path, 3)
        .map_err(|error| trace.failure(error))?;
    if before_stale != after_stale {
        return Err(trace.failure("stale Scene completion changed production Bevy resources"));
    }
    let active = production
        .active()
        .ok_or_else(|| trace.failure("active Scene/session identity is missing"))?;
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
