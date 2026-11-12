//! Seeded Scene label and identity lifecycle through the real service.

use std::{fs, path::PathBuf};

use project_protocol::{
    ProjectDeleteSceneRequest, ProjectWriteError, ProjectWriteErrorCode, ProjectWriteTarget,
};
use usd_project::SceneId;

use crate::project::{catalog::manifest_store::ManifestStore, service::ProjectApplicationService};

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
    let run_directory = artifacts::clean_run_directory(&format!("c5-{seed:016x}"))?;
    let projects_root = run_directory.join("projects");
    fs::create_dir(&projects_root).map_err(|error| format!("create C5 projects root: {error}"))?;
    let project_root = projects_root.join("Proj_T");
    let mut service = ProjectApplicationService::open(run_directory.join("workspace.json"))
        .map_err(|error| format!("open C5 service: {error}"))?;
    let fixture = fixture::create(&mut service, &projects_root)
        .map_err(|error| format!("create C5 fixture: {error}"))?;
    let fixture_ids = fixture.scenes.iter().map(|scene| scene.id).collect();
    let mut trace = Trace {
        seed,
        project_root: project_root.clone(),
        fixture_ids,
        decisions: Vec::new(),
        operations: Vec::new(),
    };
    let mut rng = DeterministicRng::seeded(seed);
    let candidates = [
        fixture.identity("Sc1.1").id,
        fixture.identity("Sc1.2.3").id,
        fixture.identities_named("Sc1.1")[1].id,
    ];
    let selected = candidates[rng.choose_index(candidates.len())];
    let selected_parent = fixture
        .scenes
        .iter()
        .find(|scene| scene.id == selected)
        .and_then(|scene| scene.parent)
        .ok_or_else(|| trace.failure("selected Scene has no parent"))?;
    let other_parent = [fixture.identity("Sc1").id, fixture.identity("Sc2").id]
        .into_iter()
        .find(|parent| *parent != selected_parent)
        .ok_or_else(|| trace.failure("no alternate parent Scene exists"))?;
    trace.decisions.push(format!("selected_scene={selected}"));
    trace
        .decisions
        .push(format!("alternate_parent={other_parent}"));

    let manifest = ManifestStore::read_validated(&project_root)
        .map_err(|error| trace.failure(format!("read C5 manifest: {error}")))?;
    let deleted_storage_key = manifest
        .scene(selected)
        .ok_or_else(|| trace.failure("selected Scene is not in manifest"))?
        .storage_key
        .as_str()
        .to_owned();
    let fresh_name = format!("M2C5Fresh{seed:016x}");

    trace
        .operations
        .push(format!("rename scene={selected} name={fresh_name}"));
    service
        .rename(
            fixture.project.id,
            ProjectWriteTarget::Scene(selected),
            &fresh_name,
        )
        .map_err(|error| trace.failure(format!("fresh rename failed: {error}")))?;
    assert_display_name(&project_root, selected, &fresh_name, &trace)?;

    trace
        .operations
        .push(format!("rename scene={selected} duplicate_name=Sc1.2"));
    service
        .rename(
            fixture.project.id,
            ProjectWriteTarget::Scene(selected),
            "Sc1.2",
        )
        .map_err(|error| trace.failure(format!("duplicate-label rename failed: {error}")))?;
    assert_display_name(&project_root, selected, "Sc1.2", &trace)?;

    trace.operations.push(format!("delete scene={selected}"));
    service
        .delete_scene(ProjectDeleteSceneRequest {
            project_id: fixture.project.id,
            scene_id: selected,
        })
        .map_err(|error| trace.failure(format!("Scene deletion failed: {error}")))?;
    let after_delete = ManifestStore::read_validated(&project_root)
        .map_err(|error| trace.failure(format!("read manifest after delete: {error}")))?;
    if after_delete.scene(selected).is_some() {
        return Err(trace.failure("deleted Scene remains registered"));
    }

    trace.operations.push(format!(
        "create parent={other_parent} deleted_storage_name={deleted_storage_key}"
    ));
    let recreated = service
        .create_scene(
            fixture.project.id,
            ProjectWriteTarget::Scene(other_parent),
            &deleted_storage_key,
        )
        .map_err(|error| trace.failure(format!("recreate Scene failed: {error}")))?;
    if recreated.scene_id == selected {
        return Err(trace.failure("recreated Scene reused the deleted SceneId"));
    }
    service
        .rename(
            fixture.project.id,
            ProjectWriteTarget::Scene(recreated.scene_id),
            "Sc1.2",
        )
        .map_err(|error| {
            trace.failure(format!("recreated duplicate-label rename failed: {error}"))
        })?;
    assert_display_name(&project_root, recreated.scene_id, "Sc1.2", &trace)?;

    trace.operations.push("delete protected root".to_owned());
    let protected = service.delete_scene(ProjectDeleteSceneRequest {
        project_id: fixture.project.id,
        scene_id: fixture.root_scene_id,
    });
    if protected
        != Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::ProtectedRootScene,
        })
    {
        return Err(trace.failure("protected root deletion was not rejected"));
    }
    Ok(())
}

fn assert_display_name(
    project_root: &std::path::Path,
    scene_id: SceneId,
    expected: &str,
    trace: &Trace,
) -> Result<(), String> {
    let manifest = ManifestStore::read_validated(project_root)
        .map_err(|error| trace.failure(format!("read renamed manifest: {error}")))?;
    let actual = manifest
        .scene(scene_id)
        .ok_or_else(|| trace.failure(format!("Scene {scene_id} missing after rename")))?
        .display_name
        .clone();
    if actual != expected {
        return Err(trace.failure(format!(
            "Scene {scene_id} display name {actual:?} != {expected:?}"
        )));
    }
    Ok(())
}
