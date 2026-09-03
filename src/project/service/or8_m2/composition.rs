//! Seeded Import/Link composition through the real Project service.

use std::{fs, path::PathBuf};

use project_protocol::{PlacementSpec, ProjectStageTarget, ProjectWriteTarget};
use tempfile::tempdir_in;
use usd_project::SceneId;

use crate::project::service::{ProjectApplicationService, ProjectModelPreparationQueue};

use super::{assets, fixture, rng::DeterministicRng};

#[path = "composition_source.rs"]
pub(super) mod composition_source;
#[path = "composition_verify.rs"]
mod composition_verify;

pub(super) fn prepare_bim_link_source(
    source: &std::path::Path,
    directory: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    composition_source::prepare_bim_link_source(source, directory)
}

#[derive(Clone, Copy, Debug)]
enum AssetKind {
    InstanceHeavy,
    DependencyAnimation,
    BimRevit,
}

impl AssetKind {
    fn label(self) -> &'static str {
        match self {
            Self::InstanceHeavy => "A",
            Self::DependencyAnimation => "B",
            Self::BimRevit => "C",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum CompositionMode {
    ImportModel,
    ImportScene,
    LinkScene,
}

impl CompositionMode {
    fn label(self) -> &'static str {
        match self {
            Self::ImportModel => "import-model",
            Self::ImportScene => "import-scene",
            Self::LinkScene => "link-scene",
        }
    }
}

#[derive(Debug)]
struct Trace {
    seed: u64,
    project_root: PathBuf,
    decisions: Vec<String>,
    operations: Vec<String>,
}

impl Trace {
    fn new(seed: u64, project_root: PathBuf) -> Self {
        Self {
            seed,
            project_root,
            decisions: Vec::new(),
            operations: Vec::new(),
        }
    }

    fn failure(&self, message: impl std::fmt::Display) -> String {
        format!(
            "{message}; seed={:#018X}; project={}; decisions={:?}; operations={:?}",
            self.seed,
            self.project_root.display(),
            self.decisions,
            self.operations
        )
    }
}

pub(super) fn run_seed(seed: u64) -> Result<(), String> {
    let run_directory = super::artifacts::clean_run_directory(&format!("c3-{seed:016x}"))?;
    let directory =
        tempdir_in(&run_directory).map_err(|error| format!("create C3 temp root: {error}"))?;
    let project_parent = directory.path().join("projects");
    fs::create_dir(&project_parent).map_err(|error| format!("create projects root: {error}"))?;
    let project_root = project_parent.join("Proj_T");
    let mut trace = Trace::new(seed, project_root.clone());
    let mut service = ProjectApplicationService::open(directory.path().join("workspace.json"))
        .map_err(|error| trace.failure(error))?;
    let fixture =
        fixture::create(&mut service, &project_parent).map_err(|error| trace.failure(error))?;
    let (bevy_assets, external_assets) = assets::default_roots();
    let dictionary =
        assets::inventory(&bevy_assets, &external_assets).map_err(|error| trace.failure(error))?;
    let sources = assets::resolve_fixtures(&dictionary, &bevy_assets, &external_assets)
        .map_err(|error| trace.failure(error))?;

    let mut rng = DeterministicRng::seeded(seed);
    let mut order = [
        AssetKind::InstanceHeavy,
        AssetKind::DependencyAnimation,
        AssetKind::BimRevit,
    ];
    for index in (1..order.len()).rev() {
        let swap = rng.choose_index(index + 1);
        trace.decisions.push(format!("order[{index}] swap={swap}"));
        order.swap(index, swap);
    }
    let canonical_parents = [fixture.identity("Sc1").id, fixture.identity("Sc2").id];
    let mut created_scenes = Vec::new();
    let preparation = ProjectModelPreparationQueue::default();

    for (ordinal, kind) in order.into_iter().enumerate() {
        let parent = choose_parent(&mut rng, &mut trace, &canonical_parents, &created_scenes);
        let (source, mode) = match kind {
            AssetKind::InstanceHeavy => {
                (sources.instance_path.clone(), CompositionMode::ImportModel)
            }
            AssetKind::DependencyAnimation => {
                let mode = choose_scene_mode(&mut rng, &mut trace, kind);
                (sources.dependency_animation_path.clone(), mode)
            }
            AssetKind::BimRevit => {
                let mode = choose_scene_mode(&mut rng, &mut trace, kind);
                let source = composition_source::prepare_bim_link_source(
                    &sources.bim_revit_path,
                    directory.path(),
                )
                .map_err(|error| trace.failure(error))?;
                trace
                    .decisions
                    .push("fixture_C_source_adapter=exact_USDC_plus_MDL_support".to_owned());
                (source, mode)
            }
        };
        let operation_id = format!("m2-c3-{seed:016x}-{ordinal}");
        let generation = ordinal as u64 + 1;
        trace.operations.push(format!(
            "asset={} mode={} parent={} operation={} generation={generation}",
            kind.label(),
            mode.label(),
            parent,
            operation_id
        ));
        match mode {
            CompositionMode::ImportModel => {
                let prepared =
                    preparation.prepare(operation_id.clone(), generation, source.clone());
                if prepared.inspection.is_err() {
                    return Err(
                        trace.failure(format!("Model preparation rejected {}", source.display()))
                    );
                }
                let response = service
                    .publish_model(
                        &preparation,
                        fixture.project.id,
                        ProjectWriteTarget::Scene(parent),
                        &source,
                        operation_id,
                        generation,
                        PlacementSpec::Default,
                    )
                    .map_err(|error| trace.failure(error))?;
                composition_verify::verify_model_placement(
                    &project_root,
                    parent,
                    response.model_id,
                    response.placement_id,
                )
                .map_err(|error| trace.failure(error))?;
                service
                    .resolve_stage_activation(
                        fixture.project.id,
                        ProjectStageTarget::Model(response.model_id),
                    )
                    .map_err(|error| trace.failure(error))?
                    .ok_or_else(|| trace.failure("published Model has no stage target"))?;
            }
            CompositionMode::ImportScene | CompositionMode::LinkScene => {
                let inspection = crate::project::scene::inspection::inspect_composition(&source)
                    .map_err(|error| trace.failure(error))?;
                let response = match mode {
                    CompositionMode::ImportModel => {
                        return Err(trace.failure("Model asset entered Scene composition branch"));
                    }
                    CompositionMode::ImportScene => service.adopt_scene(
                        fixture.project.id,
                        ProjectWriteTarget::Scene(parent),
                        &source,
                        &inspection,
                        format!("C3_{}_{}", kind.label(), ordinal),
                        operation_id,
                        generation,
                        PlacementSpec::Default,
                    ),
                    CompositionMode::LinkScene => service.link_scene(
                        fixture.project.id,
                        ProjectWriteTarget::Scene(parent),
                        &source,
                        &inspection,
                        format!("C3_{}_{}", kind.label(), ordinal),
                        operation_id,
                        generation,
                        PlacementSpec::Default,
                    ),
                }
                .map_err(|error| {
                    let closure_error =
                        crate::project::source_closure::source_closure_fingerprint(&source)
                            .err()
                            .map(|detail| detail.to_string());
                    trace.failure(format!(
                        "{error}; source_closure_diagnostic={closure_error:?}"
                    ))
                })?;
                composition_verify::verify_scene_placement(
                    &project_root,
                    parent,
                    response.scene_id,
                    response.placement_id,
                    matches!(mode, CompositionMode::LinkScene),
                )
                .map_err(|error| trace.failure(error))?;
                created_scenes.push(response.scene_id);
            }
        }
        composition_verify::verify_manifest_graph(&project_root)
            .map_err(|error| trace.failure(error))?;
    }
    Ok(())
}

fn choose_parent(
    rng: &mut DeterministicRng,
    trace: &mut Trace,
    canonical: &[SceneId; 2],
    created: &[SceneId],
) -> SceneId {
    let nested = !created.is_empty() && rng.choose_index(2) == 0;
    trace.decisions.push(format!("nested_parent={nested}"));
    if nested {
        created[rng.choose_index(created.len())]
    } else {
        canonical[rng.choose_index(canonical.len())]
    }
}

fn choose_scene_mode(
    rng: &mut DeterministicRng,
    trace: &mut Trace,
    kind: AssetKind,
) -> CompositionMode {
    if matches!(kind, AssetKind::BimRevit) {
        trace
            .decisions
            .push("scene_mode_link=true (binary BIM dependency closure)".to_owned());
        return CompositionMode::LinkScene;
    }
    let link = rng.choose_index(2) == 0;
    trace.decisions.push(format!("scene_mode_link={link}"));
    if link {
        CompositionMode::LinkScene
    } else {
        CompositionMode::ImportScene
    }
}
