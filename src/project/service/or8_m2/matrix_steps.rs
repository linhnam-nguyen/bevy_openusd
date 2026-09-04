//! Ordered in-project composition and activation steps for the OR8 M2 matrix.

use std::fs;

use project_protocol::{
    PlacementSpec, ProjectActivationCommand, ProjectReadCommand, ProjectReadRequest,
    ProjectReadResponse, ProjectStageTarget, ProjectWriteTarget,
};
use usd_project::SceneId;

use crate::project::{
    service::{
        ProjectActivationAuthority, ProjectApplicationService, ProjectModelPreparationQueue,
        ProjectStageActivation,
    },
    storage::ProjectStorageLayout,
};

use super::{matrix::Context, matrix_lifecycle, matrix_verify, rng::DeterministicRng};

#[derive(Clone, Copy)]
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

#[derive(Clone, Copy)]
enum SceneMode {
    Import,
    Link,
}

impl SceneMode {
    fn label(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Link => "link",
        }
    }
}

pub(super) fn compose_activate_and_mutate(context: &mut Context) -> Result<(), String> {
    compose(context)?;
    activate(context)?;
    matrix_lifecycle::rename_delete_recreate(context)
}

fn compose(context: &mut Context) -> Result<(), String> {
    let mut order = [
        AssetKind::InstanceHeavy,
        AssetKind::DependencyAnimation,
        AssetKind::BimRevit,
    ];
    for index in (1..order.len()).rev() {
        let swap = context.rng.choose_index(index + 1);
        context
            .trace
            .decision(format!("composition_order[{index}] swap={swap}"));
        order.swap(index, swap);
    }
    let canonical_parents = [
        context.fixture.identity("Sc1").id,
        context.fixture.identity("Sc2").id,
    ];
    let mut created_scenes = Vec::new();
    let preparation = ProjectModelPreparationQueue::default();
    for (ordinal, kind) in order.into_iter().enumerate() {
        let parent = choose_parent(
            &mut context.rng,
            &canonical_parents,
            &created_scenes,
            &mut context.trace,
        );
        let (source, mode) = source_and_mode(context, kind)?;
        let operation_id = format!("m2-c8-{:#x}-{ordinal}", context.rng.next_u64());
        let generation = ordinal as u64 + 1;
        context.trace.operation(format!(
            "compose fixture={} mode={} parent={} operation={} generation={generation}",
            kind.label(),
            mode.label(),
            parent,
            operation_id
        ));
        match kind {
            AssetKind::InstanceHeavy => publish_model(
                context,
                &preparation,
                &source,
                parent,
                operation_id,
                generation,
            )?,
            AssetKind::DependencyAnimation | AssetKind::BimRevit => {
                let inspection = crate::project::scene::inspection::inspect_composition(&source)
                    .map_err(|error| {
                        context
                            .trace
                            .failure(format!("inspect {}: {error}", kind.label()))
                    })?;
                let response = match mode {
                    SceneMode::Import => context.service.adopt_scene(
                        context.fixture.project.id,
                        ProjectWriteTarget::Scene(parent),
                        &source,
                        &inspection,
                        format!("C8_{}_{}", kind.label(), ordinal),
                        operation_id,
                        generation,
                        PlacementSpec::Default,
                    ),
                    SceneMode::Link => context.service.link_scene(
                        context.fixture.project.id,
                        ProjectWriteTarget::Scene(parent),
                        &source,
                        &inspection,
                        format!("C8_{}_{}", kind.label(), ordinal),
                        operation_id,
                        generation,
                        PlacementSpec::Default,
                    ),
                }
                .map_err(|error| {
                    context
                        .trace
                        .failure(format!("compose {}: {error}", kind.label()))
                })?;
                matrix_verify::verify_scene_placement(
                    context,
                    parent,
                    response.scene_id,
                    response.placement_id,
                    matches!(mode, SceneMode::Link),
                )?;
                created_scenes.push(response.scene_id);
            }
        }
        matrix_verify::verify_manifest_graph(context)
            .map_err(|error| context.trace.failure(error))?;
    }
    Ok(())
}

fn source_and_mode(
    context: &mut Context,
    kind: AssetKind,
) -> Result<(std::path::PathBuf, SceneMode), String> {
    match kind {
        AssetKind::InstanceHeavy => Ok((context.sources.instance_path.clone(), SceneMode::Import)),
        AssetKind::DependencyAnimation => {
            let mode = if context.rng.choose_index(2) == 0 {
                SceneMode::Link
            } else {
                SceneMode::Import
            };
            context
                .trace
                .decision(format!("fixture_B_mode={}", mode.label()));
            Ok((context.sources.dependency_animation_path.clone(), mode))
        }
        AssetKind::BimRevit => {
            context
                .trace
                .decision("fixture_C_mode=import (original USDC; missing render assets optional)");
            context.trace.decision("fixture_C_source=original_USDC");
            Ok((context.sources.bim_revit_path.clone(), SceneMode::Import))
        }
    }
}

fn publish_model(
    context: &mut Context,
    preparation: &ProjectModelPreparationQueue,
    source: &std::path::Path,
    parent: SceneId,
    operation_id: String,
    generation: u64,
) -> Result<(), String> {
    let prepared = preparation.prepare(operation_id.clone(), generation, source.to_owned());
    if prepared.inspection.is_err() {
        return Err(context
            .trace
            .failure(format!("Model preparation rejected {}", source.display())));
    }
    let response = context
        .service
        .publish_model(
            preparation,
            context.fixture.project.id,
            ProjectWriteTarget::Scene(parent),
            source,
            operation_id,
            generation,
            PlacementSpec::Default,
        )
        .map_err(|error| context.trace.failure(format!("publish Model: {error}")))?;
    matrix_verify::verify_model_placement(
        context,
        parent,
        response.model_id,
        response.placement_id,
    )?;
    context
        .service
        .resolve_stage_activation(
            context.fixture.project.id,
            ProjectStageTarget::Model(response.model_id),
        )
        .map_err(|error| context.trace.failure(format!("resolve Model: {error}")))?
        .ok_or_else(|| context.trace.failure("published Model has no stage target"))?;
    Ok(())
}

fn choose_parent(
    rng: &mut DeterministicRng,
    canonical: &[SceneId; 2],
    created: &[SceneId],
    trace: &mut super::matrix::Trace,
) -> SceneId {
    let nested = !created.is_empty() && rng.choose_index(2) == 0;
    trace.decision(format!("composition_nested_parent={nested}"));
    if nested {
        created[rng.choose_index(created.len())]
    } else {
        canonical[rng.choose_index(canonical.len())]
    }
}

fn activate(context: &mut Context) -> Result<(), String> {
    let mut eligible = [
        context.fixture.identities_named("Sc1.1")[0].id,
        context.fixture.identity("Sc1.2.3").id,
        context.fixture.identity("Sc2.1").id,
    ];
    context
        .trace
        .decision("activation_candidates=canonical_leaf_scenes");
    for index in (1..eligible.len()).rev() {
        let swap = context.rng.choose_index(index + 1);
        context
            .trace
            .decision(format!("activation_order[{index}] swap={swap}"));
        eligible.swap(index, swap);
    }
    let manifest_before = read_manifest(context)?;
    let tree_before = read_tree(&context.service, context.fixture.project.id)?;
    let expected_latest_scene = eligible[eligible.len() - 1];
    let mut authority = ProjectActivationAuthority::default();
    let mut stale_completion = None;
    for (index, scene_id) in eligible.into_iter().enumerate() {
        let generation = index as u64 + 1;
        let request_id = format!("m2-c8-activate-{index}");
        let command = ProjectActivationCommand::new(
            request_id.clone(),
            generation,
            context.fixture.project.id,
            ProjectStageTarget::Scene(scene_id),
        );
        command.validate().map_err(|error| {
            context
                .trace
                .failure(format!("invalid activation command: {error}"))
        })?;
        context.trace.operation(format!(
            "activate scene={scene_id} request={request_id} generation={generation}"
        ));
        let target = context
            .service
            .resolve_stage_activation(context.fixture.project.id, command.target.clone())
            .map_err(|error| {
                context
                    .trace
                    .failure(format!("activation resolution: {error}"))
            })?
            .ok_or_else(|| context.trace.failure("activation returned no Scene target"))?;
        if !authority.observe_request("m2-c8-session", &command) {
            return Err(context.trace.failure("activation request was not admitted"));
        }
        let target_snapshot = target.clone();
        let activation =
            ProjectStageActivation::open("m2-c8-session", &command, target).map_err(|error| {
                context
                    .trace
                    .failure(format!("activation completion: {error}"))
            })?;
        let snapshot = activation.snapshot().clone();
        if !authority.commit("m2-c8-session", &command) {
            return Err(context
                .trace
                .failure("activation completion was not committed"));
        }
        if snapshot.project_id != context.fixture.project.id
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
            return Err(context
                .trace
                .failure("active Scene/session identity is incomplete"));
        }
        context.trace.operation(format!(
            "active snapshot generation={} stage={} bim_snapshot={} bim_entities={}",
            snapshot.generation,
            snapshot.stage_path.display(),
            snapshot.bim_snapshot_id,
            snapshot.bim_entity_paths.len()
        ));
        if index == 1 {
            stale_completion = Some(command);
        }
        if read_manifest(context)? != manifest_before
            || read_tree(&context.service, context.fixture.project.id)? != tree_before
        {
            return Err(context.trace.failure("activation changed Project content"));
        }
    }
    let stale_command = stale_completion.ok_or_else(|| {
        context
            .trace
            .failure("stale activation candidate is missing")
    })?;
    if authority.commit("m2-c8-session", &stale_command) {
        return Err(context
            .trace
            .failure("stale Scene completion replaced the latest active identity"));
    }
    let active = authority.active().ok_or_else(|| {
        context
            .trace
            .failure("active Scene/session snapshot is missing")
    })?;
    if active.generation != 3 || active.target != ProjectStageTarget::Scene(expected_latest_scene) {
        return Err(context
            .trace
            .failure("latest active Scene/session identity was not retained"));
    }
    Ok(())
}

fn read_manifest(context: &Context) -> Result<Vec<u8>, String> {
    fs::read(ProjectStorageLayout::new(&context.project_root).readable_manifest_path()).map_err(
        |error| {
            context
                .trace
                .failure(format!("read Project manifest: {error}"))
        },
    )
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
