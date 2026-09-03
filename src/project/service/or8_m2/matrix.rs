//! Full OR8 M2 lifecycle matrix context and failure retention.

use std::{fs, path::PathBuf};

use crate::project::service::ProjectApplicationService;

use super::{artifacts, assets, fixture, matrix_steps, rng::DeterministicRng};

pub(super) struct Context {
    pub(super) service: ProjectApplicationService,
    pub(super) fixture: fixture::CanonicalProject,
    pub(super) sources: assets::CanonicalFixtures,
    pub(super) directory: PathBuf,
    pub(super) project_root: PathBuf,
    pub(super) export_directory: PathBuf,
    pub(super) clone_directory: PathBuf,
    pub(super) rng: DeterministicRng,
    pub(super) trace: Trace,
}

pub(super) struct Trace {
    seed: u64,
    attempt: u8,
    project_root: PathBuf,
    fixture_ids: Vec<usd_project::SceneId>,
    decisions: Vec<String>,
    operations: Vec<String>,
}

impl Trace {
    fn new(seed: u64, attempt: u8, project_root: PathBuf) -> Self {
        Self {
            seed,
            attempt,
            project_root,
            fixture_ids: Vec::new(),
            decisions: Vec::new(),
            operations: Vec::new(),
        }
    }

    pub(super) fn decision(&mut self, value: impl Into<String>) {
        self.decisions.push(value.into());
    }

    pub(super) fn operation(&mut self, value: impl Into<String>) {
        self.operations.push(value.into());
    }

    pub(super) fn failure(&self, message: impl std::fmt::Display) -> String {
        format!(
            "{message}; seed={:#018X}; attempt={}; fixture_ids={:?}; project={}; decisions={:?}; operations={:?}",
            self.seed,
            self.attempt,
            self.fixture_ids,
            self.project_root.display(),
            self.decisions,
            self.operations
        )
    }
}

pub(super) fn run_seed(seed: u64, attempt: u8) -> Result<(), String> {
    let key = format!("c8-{seed:016x}-attempt-{attempt}");
    let run_directory = artifacts::clean_run_directory(&key)?;
    let export_directory = artifacts::clean_output_directory("exports", &key)?;
    let clone_directory = artifacts::clean_output_directory("clones", &key)?;
    let result = execute(
        seed,
        attempt,
        run_directory.clone(),
        export_directory,
        clone_directory,
    );
    if let Err(error) = &result {
        let failure_path = run_directory.join("failure.txt");
        let _ = fs::write(&failure_path, error);
        eprintln!("C8 retained failure artifact: {}", failure_path.display());
    }
    result
}

pub(super) fn clean_previous_attempts(seed: u64) -> Result<(), String> {
    for attempt in 1..=4 {
        let key = format!("c8-{seed:016x}-attempt-{attempt}");
        artifacts::clean_run_directory(&key)?;
        artifacts::clean_output_directory("exports", &key)?;
        artifacts::clean_output_directory("clones", &key)?;
    }
    Ok(())
}

fn execute(
    seed: u64,
    attempt: u8,
    directory: PathBuf,
    export_directory: PathBuf,
    clone_directory: PathBuf,
) -> Result<(), String> {
    let projects_root = directory.join("projects");
    fs::create_dir(&projects_root).map_err(|error| format!("create C8 projects root: {error}"))?;
    let project_root = projects_root.join("Proj_T");
    let mut service = ProjectApplicationService::open(directory.join("workspace.json"))
        .map_err(|error| format!("open C8 service: {error}"))?;
    let fixture = fixture::create(&mut service, &projects_root)
        .map_err(|error| format!("create C8 fixture: {error}"))?;
    let (bevy_assets, external_assets) = assets::default_roots();
    let dictionary = assets::inventory(&bevy_assets, &external_assets)?;
    let sources = assets::resolve_fixtures(&dictionary, &bevy_assets, &external_assets)?;
    let fixture_ids = fixture.scenes.iter().map(|scene| scene.id).collect();
    let mut context = Context {
        service,
        fixture,
        sources,
        directory,
        project_root: project_root.clone(),
        export_directory,
        clone_directory,
        rng: DeterministicRng::seeded(seed),
        trace: Trace::new(seed, attempt, project_root),
    };
    context.trace.fixture_ids = fixture_ids;
    context.trace.decision(format!(
        "fixtures=A:{} B:{} C:{}",
        context.sources.instance_heavy.asset_key,
        context.sources.dependency_animation.asset_key,
        context.sources.bim_revit.asset_key
    ));
    context
        .trace
        .decision(format!("attempt_directory={}", context.directory.display()));
    matrix_steps::compose_activate_and_mutate(&mut context)?;
    super::matrix_persistence::export_roundtrip(&mut context)?;
    super::matrix_persistence::clone_and_validate(&mut context)
}
