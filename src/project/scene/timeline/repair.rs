//! Read-only legacy timeline inspection and explicit non-destructive
//! transactional repair for Model wrappers, adopted Scenes, and composed parents.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use openusd::{
    sdf,
    usd::{InitialLoadSet, Stage},
};
use usd_project::{ModelId, ProjectManifestV1, SceneId, SceneMemberTarget};
use uuid::Uuid;

use super::domain::{
    aggregate_member_domains, has_intentional_authored_range, map_child_domain_to_parent,
    recompute_scene_stage_timeline, resolve_wrapper_source_path,
};
use super::propagation::{all_scenes_child_first, read_all_scene_members};
use super::publication::publish_staged_layers_atomic;
use super::{
    MODEL_ROOT_PRIM, PROJECT_METADATA_DIRECTORY, ProjectTimelineRepairPlan,
    ProjectTimelineRepairReport, TRANSACTIONS_DIRECTORY, TimelineRepairEntry,
    TimelineRepairTarget, TimelineStageOverrides,
};
use crate::document::stages::metadata::{
    StageTimeMetadata, TimeCodeRange, inspect_source_time_metadata, inspect_stage_time_metadata,
};
use crate::project::scene::authoring::{SCENE_ROOT_PRIM, scene_path, validate_scene_file};
use crate::project::storage::ProjectStorageLayout;

/// Inspect a Project's current working tree read-only and detect legacy Model
/// wrappers, adopted Scene wrappers, and composed parent Scenes whose timeline
/// metadata is missing or out-of-date. Never mutates files or Git history.
pub(crate) fn inspect_project_timeline_repairs(
    project_root: &Path,
    manifest: &ProjectManifestV1,
) -> Result<ProjectTimelineRepairPlan> {
    let layout = ProjectStorageLayout::new(project_root);
    let mut plan = ProjectTimelineRepairPlan::default();
    let mut projected_models: HashMap<ModelId, StageTimeMetadata> = HashMap::new();
    let mut projected_scenes: HashMap<SceneId, StageTimeMetadata> = HashMap::new();
    let mut overrides = TimelineStageOverrides::default();
    overrides.populate_manifest_targets(project_root, manifest);

    // 1. Inspect Model wrappers against their localized source asset.
    for model in &manifest.models {
        let wrapper_path = layout.canonical_model_wrapper_path(model);
        ensure!(
            wrapper_path.is_file(),
            "required Model wrapper {} is missing",
            model.id
        );
        let wrapper_stage = Stage::builder()
            .load(InitialLoadSet::LoadNone)
            .open(wrapper_path.to_string_lossy().as_ref())
            .with_context(|| format!("open Model wrapper {} for timeline inspection", model.id))?;
        let current = inspect_stage_time_metadata(&wrapper_stage, false)?;
        let mut effective = current;

        let source_path =
            resolve_wrapper_source_path(&wrapper_stage, &wrapper_path, MODEL_ROOT_PRIM)?
                .with_context(|| format!("Model wrapper {} has no source reference", model.id))?;
        ensure!(
            source_path.is_file(),
            "Model wrapper source is missing: {}",
            source_path.display()
        );
        let source_meta = inspect_source_time_metadata(&source_path)?;
        if let Some((entry, next)) = plan_wrapper_metadata_repair(
            &wrapper_stage,
            TimelineRepairTarget::Model(model.id),
            &wrapper_path,
            current,
            source_meta,
        )? {
            plan.entries.push(entry);
            effective = next;
        }
        projected_models.insert(model.id, effective);
    }

    // 2. Inspect Scenes in child-first topological order so parents observe
    //    repaired child ranges without mutating disk.
    let scene_members = read_all_scene_members(project_root, manifest)?;
    let ordered_scenes = all_scenes_child_first(&scene_members, manifest)?;

    for scene_id in ordered_scenes {
        let layer_path = overrides
            .scene_targets
            .get(&scene_id)
            .cloned()
            .unwrap_or_else(|| scene_path(project_root, scene_id));
        ensure!(
            layer_path.is_file(),
            "required Scene layer {scene_id} is missing"
        );
        let stage = Stage::builder()
            .load(InitialLoadSet::LoadNone)
            .open(layer_path.to_string_lossy().as_ref())
            .with_context(|| format!("open Scene {scene_id} for timeline inspection"))?;
        let current = inspect_stage_time_metadata(&stage, false)?;
        let members = scene_members
            .get(&scene_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);

        let source_path = resolve_wrapper_source_path(&stage, &layer_path, SCENE_ROOT_PRIM)?;
        let mut effective = current;
        let mut entry = None;
        if let Some(source_path) = source_path {
            let source = inspect_source_time_metadata(&source_path)?;
            if let Some((mut fields, next)) = plan_wrapper_metadata_repair(
                &stage,
                TimelineRepairTarget::Scene(scene_id),
                &layer_path,
                current,
                source,
            )? {
                // Scene bounds are always calculated by the shared aggregator;
                // only the missing rate/FPS fields come from wrapper repair.
                fields.repair_range = false;
                fields.proposed_range = fields.current_range;
                effective.time_codes_per_second = next.time_codes_per_second;
                effective.frames_per_second = next.frames_per_second;
                entry = Some(fields);
            }
        }
        let parent_tcps = effective.time_codes_per_second;
        let computed = aggregate_member_domains(
            &stage,
            project_root,
            &layer_path,
            members,
            parent_tcps,
            &overrides,
            |member| {
                match member.target {
                    SceneMemberTarget::Scene(child_id) => {
                        projected_scenes.get(&child_id).copied()
                    }
                    SceneMemberTarget::Model(child_id) => {
                        projected_models.get(&child_id).copied()
                    }
                }
                .with_context(|| {
                    format!("unknown projected playback domain for member {}", member.id)
                })
            },
        )?;
        let aggregated = if has_intentional_authored_range(&stage)? {
            current.range
        } else {
            computed
        };
        let needs_update = current.range != aggregated;
        if needs_update
            || entry
                .as_ref()
                .is_some_and(|fields| fields.repair_time_codes_per_second || fields.repair_frames_per_second)
        {
            let mut fields = entry.unwrap_or_else(|| TimelineRepairEntry {
                target: TimelineRepairTarget::Scene(scene_id),
                layer_path,
                current_range: current.range.map(|r| (r.start, r.end)),
                proposed_range: None,
                current_time_codes_per_second: current.time_codes_per_second,
                proposed_time_codes_per_second: parent_tcps,
                proposed_frames_per_second: effective.frames_per_second,
                repair_range: false,
                repair_time_codes_per_second: false,
                repair_frames_per_second: false,
                reason: "generated Scene playback metadata".to_owned(),
            });
            fields.repair_range = needs_update;
            fields.proposed_range = aggregated.map(|r| (r.start, r.end));
            plan.entries.push(fields);
        }
        projected_scenes.insert(
            scene_id,
            StageTimeMetadata {
                range: aggregated,
                time_codes_per_second: parent_tcps,
                frames_per_second: effective.frames_per_second,
            },
        );
    }

    Ok(plan)
}

/// Explicitly and transactionally repair legacy timeline metadata in the
/// Project's current working tree. Idempotent and non-destructive; never
/// rewrites Git history.
pub(crate) fn repair_project_timelines_atomic(
    project_root: &Path,
    manifest: &ProjectManifestV1,
) -> Result<ProjectTimelineRepairReport> {
    let plan = inspect_project_timeline_repairs(project_root, manifest)?;
    if plan.is_empty() {
        return Ok(ProjectTimelineRepairReport::default());
    }

    let scene_members = read_all_scene_members(project_root, manifest)?;
    let transaction_dir = project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join(TRANSACTIONS_DIRECTORY)
        .join(format!("timeline-repair-{}", Uuid::new_v4()));
    fs::create_dir_all(&transaction_dir)
        .context("create timeline repair transaction directory")?;

    let mut staged_overrides = TimelineStageOverrides::default();
    staged_overrides.populate_manifest_targets(project_root, manifest);
    let mut staged_files: Vec<(TimelineRepairTarget, PathBuf, PathBuf)> = Vec::new();

    let prepare_result = (|| -> Result<()> {
        for (index, entry) in plan.entries.iter().enumerate() {
            match entry.target {
                TimelineRepairTarget::Model(model_id) => {
                    let stage = Stage::builder()
                        .load(InitialLoadSet::LoadNone)
                        .open(entry.layer_path.to_string_lossy().as_ref())
                        .with_context(|| format!("open Model wrapper {model_id} for repair"))?;
                    apply_wrapper_repair(&stage, entry)?;
                    let temp_path = transaction_dir.join(format!("repair-{index}-model.tmp.usda"));
                    stage
                        .root_layer()
                        .export(temp_path.to_string_lossy().as_ref())
                        .with_context(|| format!("export repaired Model wrapper {model_id}"))?;
                    staged_overrides.models.insert(model_id, temp_path.clone());
                    staged_files.push((entry.target, temp_path, entry.layer_path.clone()));
                }
                TimelineRepairTarget::Scene(scene_id) => {
                    let stage = Stage::open(entry.layer_path.to_string_lossy().as_ref())
                        .with_context(|| format!("open Scene {scene_id} for repair"))?;
                    let members = scene_members
                        .get(&scene_id)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    // Apply only missing rate/FPS fields before calculating the
                    // generated range, so a newly written range cannot be
                    // mistaken for a deliberate user override.
                    let mut fields = entry.clone();
                    fields.repair_range = false;
                    apply_wrapper_repair(&stage, &fields)?;
                    recompute_scene_stage_timeline(
                        &stage,
                        project_root,
                        &entry.layer_path,
                        members,
                        &staged_overrides,
                    )?;
                    let temp_path = transaction_dir.join(format!("repair-{index}-scene.tmp.usda"));
                    stage
                        .root_layer()
                        .export(temp_path.to_string_lossy().as_ref())
                        .with_context(|| format!("export repaired Scene {scene_id}"))?;
                    validate_scene_file(&temp_path, scene_id, members)?;
                    staged_overrides.scenes.insert(scene_id, temp_path.clone());
                    staged_files.push((entry.target, temp_path, entry.layer_path.clone()));
                }
            }
        }
        Ok(())
    })();

    if let Err(error) = prepare_result {
        let _ = fs::remove_dir_all(&transaction_dir);
        return Err(error);
    }

    let publish_pairs: Vec<(usize, PathBuf, PathBuf)> = staged_files
        .iter()
        .enumerate()
        .map(|(i, (_, temp, final_path))| (i, temp.clone(), final_path.clone()))
        .collect();
    publish_staged_layers_atomic(&transaction_dir, &publish_pairs)?;
    let _ = fs::remove_dir_all(&transaction_dir);

    let mut report = ProjectTimelineRepairReport::default();
    for (target, _, _) in staged_files {
        match target {
            TimelineRepairTarget::Model(model_id) => report.repaired_models.push(model_id),
            TimelineRepairTarget::Scene(scene_id) => report.repaired_scenes.push(scene_id),
        }
    }
    Ok(report)
}

/// Ambiguous authored legacy values are preserved. An explicit migration of
/// already authored, differing wrapper bounds requires a separate decision.
fn plan_wrapper_metadata_repair(
    wrapper: &Stage,
    target: TimelineRepairTarget,
    path: &Path,
    current: StageTimeMetadata,
    source: StageTimeMetadata,
) -> Result<Option<(TimelineRepairEntry, StageTimeMetadata)>> {
    let has_start = wrapper.has_authored_stage_metadata(sdf::FieldKey::StartTimeCode.as_str())?;
    let has_end = wrapper.has_authored_stage_metadata(sdf::FieldKey::EndTimeCode.as_str())?;
    // Partially authored ranges are ambiguous, not permission to overwrite.
    let repair_range = !has_start && !has_end && source.range.is_some();
    let repair_rate =
        !wrapper.has_authored_stage_metadata(sdf::FieldKey::TimeCodesPerSecond.as_str())?
            && ((!has_start && !has_end)
                || current.time_codes_per_second == source.time_codes_per_second);
    let repair_fps = current.frames_per_second.is_none() && source.frames_per_second.is_some();
    if !repair_range && !repair_rate && !repair_fps {
        return Ok(None);
    }
    let rate = if repair_rate {
        source.time_codes_per_second
    } else {
        current.time_codes_per_second
    };
    let range = if repair_range {
        source
            .range
            .map(|range| {
                map_child_domain_to_parent(
                    range,
                    source.time_codes_per_second,
                    rate,
                    sdf::LayerOffset::IDENTITY,
                )
            })
            .transpose()?
    } else {
        current.range
    };
    let fps = if repair_fps {
        source.frames_per_second
    } else {
        current.frames_per_second
    };
    let next = StageTimeMetadata {
        range,
        time_codes_per_second: rate,
        frames_per_second: fps,
    };
    Ok(Some((
        TimelineRepairEntry {
            target,
            layer_path: path.to_owned(),
            current_range: current.range.map(|r| (r.start, r.end)),
            proposed_range: range.map(|r| (r.start, r.end)),
            current_time_codes_per_second: current.time_codes_per_second,
            proposed_time_codes_per_second: rate,
            proposed_frames_per_second: fps,
            repair_range,
            repair_time_codes_per_second: repair_rate,
            repair_frames_per_second: repair_fps,
            reason: "missing wrapper playback metadata; authored fields preserved".to_owned(),
        },
        next,
    )))
}

fn apply_wrapper_repair(stage: &Stage, entry: &TimelineRepairEntry) -> Result<()> {
    if entry.repair_range {
        ensure!(
            !stage.has_authored_stage_metadata(sdf::FieldKey::StartTimeCode.as_str())?
                && !stage.has_authored_stage_metadata(sdf::FieldKey::EndTimeCode.as_str())?,
            "wrapper playback range changed during repair"
        );
        let (start, end) = entry.proposed_range.context("repair range missing")?;
        let range = TimeCodeRange::new(start, end)?;
        stage.set_start_time_code(range.start)?;
        stage.set_end_time_code(range.end)?;
    }
    if entry.repair_time_codes_per_second {
        ensure!(
            !stage.has_authored_stage_metadata(sdf::FieldKey::TimeCodesPerSecond.as_str())?,
            "wrapper timeCodesPerSecond changed during repair"
        );
        stage.set_time_codes_per_second(entry.proposed_time_codes_per_second)?;
    }
    if entry.repair_frames_per_second {
        ensure!(
            !stage.has_authored_stage_metadata(sdf::FieldKey::FramesPerSecond.as_str())?,
            "wrapper framesPerSecond changed during repair"
        );
        stage.set_frames_per_second(
            entry
                .proposed_frames_per_second
                .context("repair FPS missing")?,
        )?;
    }
    Ok(())
}
