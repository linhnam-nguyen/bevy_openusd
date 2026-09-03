//! Export and normal service reimport for M2-C6.

use std::{fs, path::PathBuf};

use openusd::usd::{InitialLoadSet, PrimPredicate, Stage};
use project_protocol::{
    LocalSelectionToken, PlacementSpec, ProjectExportSceneRequest, ProjectWriteTarget,
};
use usd_project::{SceneId, SceneMemberTarget};

use crate::project::{
    catalog::manifest_store::ManifestStore,
    scene::{
        authoring::{read_scene_members, scene_path},
        inspection::inspect_composition,
    },
    service::ProjectApplicationService,
};

use super::{artifacts, fixture, rng::DeterministicRng};

#[derive(Debug)]
struct Trace {
    seed: u64,
    project_root: PathBuf,
    fixture_ids: Vec<SceneId>,
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
    let run_directory = artifacts::clean_run_directory(&format!("c6-{seed:016x}"))?;
    let export_directory =
        artifacts::clean_output_directory("exports", &format!("c6-{seed:016x}"))?;
    let projects_root = run_directory.join("projects");
    fs::create_dir(&projects_root).map_err(|error| format!("create C6 projects root: {error}"))?;
    let project_root = projects_root.join("Proj_T");
    let mut service = ProjectApplicationService::open(run_directory.join("workspace.json"))
        .map_err(|error| format!("open C6 service: {error}"))?;
    let fixture = fixture::create(&mut service, &projects_root)
        .map_err(|error| format!("create C6 fixture: {error}"))?;
    let fixture_ids = fixture.scenes.iter().map(|scene| scene.id).collect();
    let mut trace = Trace {
        seed,
        project_root: project_root.clone(),
        fixture_ids,
        operations: Vec::new(),
    };
    let mut eligible = [
        fixture.identities_named("Sc1.1")[0].id,
        fixture.identity("Sc1.2.3").id,
        fixture.identity("Sc2.1").id,
    ];
    let mut rng = DeterministicRng::seeded(seed);
    for index in (1..eligible.len()).rev() {
        eligible.swap(index, rng.choose_index(index + 1));
    }
    let source_scene = eligible[0];
    let target_scene = eligible[1];
    trace.operations.push(format!(
        "roundtrip_selection source={source_scene} target={target_scene} candidates={eligible:?}"
    ));
    let destination = export_directory.join("selected-scene.usdz");
    trace.operations.push(format!(
        "export scene={source_scene} path={}",
        destination.display()
    ));
    let export = service
        .export_scene(
            ProjectExportSceneRequest {
                project_id: fixture.project.id,
                scene_id: source_scene,
                destination: LocalSelectionToken::new("m2-c6-export"),
            },
            &destination,
        )
        .map_err(|error| trace.failure(format!("Scene export failed: {error}")))?;
    if export.scene_id != source_scene || !destination.is_file() {
        return Err(trace.failure("export response or destination is invalid"));
    }
    verify_export(&destination).map_err(|error| trace.failure(error))?;

    let inspection = inspect_composition(&destination)
        .map_err(|error| trace.failure(format!("inspect exported Scene: {error}")))?;
    trace.operations.push(format!(
        "reimport source={} target={} mode=normal-adopt",
        destination.display(),
        target_scene
    ));
    let imported = service
        .adopt_scene(
            fixture.project.id,
            ProjectWriteTarget::Scene(target_scene),
            &destination,
            &inspection,
            "C6_Roundtrip".to_owned(),
            format!("m2-c6-{seed:016x}-reimport"),
            2,
            PlacementSpec::Default,
        )
        .map_err(|error| trace.failure(format!("normal Scene reimport failed: {error}")))?;
    if imported.scene_id == source_scene {
        return Err(trace.failure("reimport reused the exported Scene identity"));
    }
    verify_reimport(&project_root, target_scene, imported.scene_id, &trace)?;
    Ok(())
}

fn verify_export(path: &std::path::Path) -> Result<(), String> {
    let file = fs::File::open(path).map_err(|error| format!("open exported archive: {error}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("read exported USDZ archive: {error}"))?;
    if archive.is_empty() {
        return Err("exported USDZ archive is empty".to_owned());
    }
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("read USDZ entry {index}: {error}"))?;
        let name = entry.name();
        if name.starts_with('/') || name.split('/').any(|part| part == "..") {
            return Err(format!("exported archive contains unsafe entry {name:?}"));
        }
    }
    drop(archive);
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(path.to_string_lossy().as_ref())
        .map_err(|error| format!("open exported USDZ stage: {error}"))?;
    stage
        .traverse(PrimPredicate::DEFAULT, |_| {})
        .map_err(|error| format!("traverse exported USDZ stage: {error}"))?;
    if !stage.composition_errors().is_empty() {
        return Err(format!(
            "exported USDZ has composition errors: {:?}",
            stage.composition_errors()
        ));
    }
    Ok(())
}

fn verify_reimport(
    project_root: &std::path::Path,
    parent: SceneId,
    scene_id: SceneId,
    trace: &Trace,
) -> Result<(), String> {
    let members = read_scene_members(&scene_path(project_root, parent), parent)
        .map_err(|error| trace.failure(format!("read reimport parent: {error}")))?;
    if !members
        .iter()
        .any(|member| member.target == SceneMemberTarget::Scene(scene_id))
    {
        return Err(trace.failure("reimported Scene is not placed in target"));
    }
    let manifest = ManifestStore::read_validated(project_root)
        .map_err(|error| trace.failure(format!("read reimport manifest: {error}")))?;
    let entry = manifest
        .scene(scene_id)
        .ok_or_else(|| trace.failure("reimported Scene is absent from manifest"))?;
    if entry.storage_key.as_str() != "C6_Roundtrip" {
        return Err(trace.failure("reimport storage identity is not service-derived"));
    }
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(
            scene_path(project_root, scene_id)
                .to_string_lossy()
                .as_ref(),
        )
        .map_err(|error| trace.failure(format!("open reimported Scene: {error}")))?;
    stage
        .traverse(PrimPredicate::DEFAULT, |_| {})
        .map_err(|error| trace.failure(format!("traverse reimported Scene: {error}")))?;
    Ok(())
}
