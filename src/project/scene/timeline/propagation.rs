//! Composition graph traversal and staged ancestor timeline propagation for
//! imports, membership updates, saves, and member retiming.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use openusd::{sdf, usd::Stage, vt::Value};
use usd_project::{
    ModelId, ProjectManifestV1, SceneId, SceneMember, SceneMemberId, SceneMemberTarget,
};
use uuid::Uuid;

use super::domain::{apply_generated_timeline, recompute_scene_stage_timeline};
use super::publication::publish_staged_layers_atomic;
use super::{
    PROJECT_METADATA_DIRECTORY, TRANSACTIONS_DIRECTORY, TimelineStageOverrides,
    TimelineUpdateOutcome,
};
use crate::project::scene::authoring::{
    REFERENCES_FIELD, read_scene_members, scene_member_path_for_stage, scene_path,
    validate_scene_file,
};
use crate::project::storage::ProjectStorageLayout;

/// Compute all transitive ancestor Scenes of `seed_scenes` in child-first
/// topological order (immediate parents before grandparents, up to Project root).
///
/// Fails if a composition cycle is encountered among the affected ancestors.
pub(crate) fn affected_ancestors_child_first(
    scene_members: &HashMap<SceneId, Vec<SceneMember>>,
    seed_scenes: &[SceneId],
) -> Result<Vec<SceneId>> {
    let mut parents_of: HashMap<SceneId, Vec<SceneId>> = HashMap::new();
    for (&parent_id, members) in scene_members {
        for member in members {
            if let SceneMemberTarget::Scene(child_id) = member.target {
                parents_of.entry(child_id).or_default().push(parent_id);
            }
        }
    }

    let mut ancestors = HashSet::new();
    let mut queue: VecDeque<SceneId> = seed_scenes.iter().copied().collect();
    while let Some(child_id) = queue.pop_front() {
        for &parent_id in parents_of.get(&child_id).into_iter().flatten() {
            if ancestors.insert(parent_id) {
                queue.push_back(parent_id);
            }
        }
    }
    if ancestors.is_empty() {
        return Ok(Vec::new());
    }

    // Kahn's topological sort over `ancestors` in child-first order:
    // edge child -> parent means `child` must be processed before `parent`.
    let mut in_degree: HashMap<SceneId, usize> =
        ancestors.iter().map(|&scene_id| (scene_id, 0)).collect();
    let mut dependents: HashMap<SceneId, Vec<SceneId>> = HashMap::new();
    for &parent_id in &ancestors {
        for member in scene_members.get(&parent_id).into_iter().flatten() {
            if let SceneMemberTarget::Scene(child_id) = member.target
                && ancestors.contains(&child_id)
            {
                dependents.entry(child_id).or_default().push(parent_id);
                *in_degree.entry(parent_id).or_insert(0) += 1;
            }
        }
    }

    let mut ready: Vec<SceneId> = in_degree
        .iter()
        .filter_map(|(&scene_id, &degree)| (degree == 0).then_some(scene_id))
        .collect();
    ready.sort();
    let mut ready_queue = VecDeque::from(ready);
    let mut ordered = Vec::with_capacity(ancestors.len());

    while let Some(scene_id) = ready_queue.pop_front() {
        ordered.push(scene_id);
        let mut next_ready = Vec::new();
        for &parent_id in dependents.get(&scene_id).into_iter().flatten() {
            if let Some(degree) = in_degree.get_mut(&parent_id) {
                *degree -= 1;
                if *degree == 0 {
                    next_ready.push(parent_id);
                }
            }
        }
        next_ready.sort();
        ready_queue.extend(next_ready);
    }

    ensure!(
        ordered.len() == ancestors.len(),
        "Project Scene composition cycle detected during ancestor timeline propagation"
    );
    Ok(ordered)
}

/// Ancestor candidates prepared without changing canonical layers. The enclosing
/// project publication owns the transaction directory and its recovery preimages.
pub(crate) struct PreparedAncestorTimelines {
    directory: PathBuf,
    layers: Vec<(SceneId, PathBuf, PathBuf)>,
}

impl PreparedAncestorTimelines {
    pub(crate) fn publish(self) -> Result<Vec<SceneId>> {
        if self.layers.is_empty() {
            return Ok(Vec::new());
        }
        publish_staged_layers_atomic(&self.directory, &self.layers)?;
        Ok(self.layers.into_iter().map(|(id, _, _)| id).collect())
    }
}

/// Shared staging helper that recomputes, exports, and validates every affected
/// ancestor in child-first order into `directory`.
fn stage_ancestor_updates(
    project_root: &Path,
    scene_members: &HashMap<SceneId, Vec<SceneMember>>,
    ancestors: &[SceneId],
    directory: &Path,
    overrides: &mut TimelineStageOverrides,
    staged: &mut Vec<(SceneId, PathBuf, PathBuf)>,
) -> Result<()> {
    for &ancestor_id in ancestors {
        let final_path = overrides
            .scene_targets
            .get(&ancestor_id)
            .cloned()
            .unwrap_or_else(|| scene_path(project_root, ancestor_id));
        let members = scene_members
            .get(&ancestor_id)
            .context("missing required ancestor graph node")?;
        let stage = Stage::open(final_path.to_string_lossy().as_ref())
            .with_context(|| format!("open ancestor Scene {ancestor_id} for timeline update"))?;
        let outcome =
            recompute_scene_stage_timeline(&stage, project_root, &final_path, members, overrides)?;
        if matches!(outcome, TimelineUpdateOutcome::Updated(_)) {
            let temporary = directory.join(format!("{ancestor_id}.tmp.usda"));
            stage
                .root_layer()
                .export(temporary.to_string_lossy().as_ref())
                .with_context(|| format!("export ancestor Scene {ancestor_id}"))?;
            validate_scene_file(&temporary, ancestor_id, members)?;
            overrides.scenes.insert(ancestor_id, temporary.clone());
            staged.push((ancestor_id, temporary, final_path));
        }
    }
    Ok(())
}

/// Prepare every affected ancestor using staged child and immediate-parent
/// layers. No ancestor is written canonically until all candidates validate.
pub(crate) fn prepare_ancestor_timelines(
    project_root: &Path,
    manifest: &ProjectManifestV1,
    changed_scenes: &[SceneId],
    mut overrides: TimelineStageOverrides,
    directory: &Path,
) -> Result<PreparedAncestorTimelines> {
    let mut prepared = PreparedAncestorTimelines {
        directory: directory.to_owned(),
        layers: Vec::new(),
    };
    if changed_scenes.is_empty() {
        return Ok(prepared);
    }
    overrides.populate_manifest_targets(project_root, manifest);
    let scene_members = read_all_scene_members_with_overrides(project_root, manifest, &overrides)?;
    all_scenes_child_first(&scene_members, manifest)?;
    let ancestors = affected_ancestors_child_first(&scene_members, changed_scenes)?;
    if ancestors.is_empty() {
        return Ok(prepared);
    }
    fs::create_dir_all(directory).context("create staged ancestor timeline directory")?;
    stage_ancestor_updates(
        project_root,
        &scene_members,
        &ancestors,
        directory,
        &mut overrides,
        &mut prepared.layers,
    )?;
    Ok(prepared)
}

/// Prepare changed parent membership and all affected ancestors in a single
/// child-first group before deleting definitions or publishing the manifest.
pub(crate) fn prepare_scene_member_timelines(
    project_root: &Path,
    manifest: &ProjectManifestV1,
    replacements: &HashMap<SceneId, Vec<SceneMember>>,
    directory: &Path,
) -> Result<PreparedAncestorTimelines> {
    let mut prepared = PreparedAncestorTimelines {
        directory: directory.to_owned(),
        layers: Vec::new(),
    };
    if replacements.is_empty() {
        return Ok(prepared);
    }
    let mut overrides = TimelineStageOverrides::default();
    overrides.populate_manifest_targets(project_root, manifest);
    let mut graph = read_all_scene_members(project_root, manifest)?;
    for (&id, members) in replacements {
        ensure!(
            graph.contains_key(&id),
            "replacement Scene {id} is not in candidate manifest"
        );
        graph.insert(id, members.clone());
    }
    let seeds: Vec<_> = replacements.keys().copied().collect();
    let order = all_scenes_child_first(&graph, manifest)?;
    let affected: HashSet<_> = affected_ancestors_child_first(&graph, &seeds)?
        .into_iter()
        .chain(seeds)
        .collect();
    fs::create_dir_all(directory)?;
    for id in order.into_iter().filter(|id| affected.contains(id)) {
        let path = overrides
            .scene_targets
            .get(&id)
            .cloned()
            .unwrap_or_else(|| scene_path(project_root, id));
        let members = graph.get(&id).context("missing affected Scene")?;
        let temp = directory.join(format!("{id}.tmp.usda"));
        if replacements.contains_key(&id) {
            crate::project::scene::members::lifecycle::prepare_scene_members_replacement(
                &path,
                &temp,
                project_root,
                id,
                members,
                &overrides,
            )?;
        } else {
            let stage = Stage::open(path.to_string_lossy().as_ref())?;
            if !matches!(
                recompute_scene_stage_timeline(&stage, project_root, &path, members, &overrides)?,
                TimelineUpdateOutcome::Updated(_)
            ) {
                continue;
            }
            stage.root_layer().export(temp.to_string_lossy().as_ref())?;
            validate_scene_file(&temp, id, members)?;
        }
        overrides.scenes.insert(id, temp.clone());
        prepared.layers.push((id, temp, path));
    }
    Ok(prepared)
}

/// Standalone propagation entry point for callers that already own the
/// canonical mutation barrier. Mutations preparing new layers should use the
/// staged preparation interface instead of re-reading newly published layers.
pub(crate) fn propagate_ancestor_timelines_atomic(
    project_root: &Path,
    manifest: &ProjectManifestV1,
    changed_scenes: &[SceneId],
) -> Result<Vec<SceneId>> {
    let directory = project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join(TRANSACTIONS_DIRECTORY)
        .join(format!("timeline-propagate-{}", Uuid::new_v4()));
    let prepared = match prepare_ancestor_timelines(
        project_root,
        manifest,
        changed_scenes,
        TimelineStageOverrides::default(),
        &directory,
    ) {
        Ok(prepared) => prepared,
        Err(error) => {
            let _ = fs::remove_dir_all(&directory);
            return Err(error);
        }
    };
    let updated = prepared.publish()?;
    let _ = fs::remove_dir_all(&directory);
    Ok(updated)
}

/// Save a managed Scene and its generated timeline together with all affected
/// ancestors. The candidate layer must be a sibling of the canonical layer so
/// existing relative asset paths retain their meaning during inspection.
pub(crate) fn save_scene_with_timelines_atomic(
    edited_stage: &Stage,
    project_root: &Path,
    manifest: &ProjectManifestV1,
    scene_id: SceneId,
    destination: &Path,
) -> Result<Vec<SceneId>> {
    let transaction_dir = project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join(TRANSACTIONS_DIRECTORY)
        .join(format!("scene-save-{}", Uuid::new_v4()));
    fs::create_dir_all(&transaction_dir)?;
    let candidate = destination.with_file_name(format!(".scene-save-{}.tmp.usda", Uuid::new_v4()));
    let mut staged = Vec::new();
    let mut seed_outcome = TimelineUpdateOutcome::PreservedUserRange;
    let prepared = (|| -> Result<()> {
        let mut overrides = TimelineStageOverrides::default();
        overrides.populate_manifest_targets(project_root, manifest);
        usd_bevy::authoring::save_stage_as(edited_stage, candidate.to_string_lossy().as_ref())?;
        let members = read_scene_members(&candidate, scene_id)?;
        let stage = Stage::open(candidate.to_string_lossy().as_ref())?;
        seed_outcome = recompute_scene_stage_timeline(
            &stage,
            project_root,
            destination,
            &members,
            &overrides,
        )?;
        stage
            .root_layer()
            .export(candidate.to_string_lossy().as_ref())?;
        validate_scene_file(&candidate, scene_id, &members)?;
        let mut graph = read_all_scene_members(project_root, manifest)?;
        graph.insert(scene_id, members);
        all_scenes_child_first(&graph, manifest)?;
        let ancestors = affected_ancestors_child_first(&graph, &[scene_id])?;
        overrides.scenes.insert(scene_id, candidate.clone());
        staged.push((scene_id, candidate.clone(), destination.to_owned()));
        stage_ancestor_updates(
            project_root,
            &graph,
            &ancestors,
            &transaction_dir,
            &mut overrides,
            &mut staged,
        )?;
        Ok(())
    })();
    if let Err(error) = prepared {
        let _ = fs::remove_file(&candidate);
        let _ = fs::remove_dir_all(&transaction_dir);
        return Err(error);
    }
    publish_staged_layers_atomic(&transaction_dir, &staged)?;
    let _ = fs::remove_dir_all(&transaction_dir);
    if let TimelineUpdateOutcome::Updated(range) = seed_outcome {
        // Canonical publication has succeeded. Keep the active authored stage
        // in sync with the same generated metadata, without changing history.
        if let Err(error) =
            apply_generated_timeline(edited_stage, edited_stage.time_codes_per_second(), range)
        {
            log::warn!(
                "canonical Scene saved; active timeline refresh requires reload: {error:#}"
            );
        }
    }
    Ok(staged.into_iter().map(|(id, _, _)| id).collect())
}

/// Update one member reference's `LayerOffset` (`offset`, `scale`) on a parent
/// Scene, recompute that parent's playback domain, and propagate atomically
/// through all affected ancestors up to Project root.
pub(crate) fn retime_scene_member_atomic(
    project_root: &Path,
    manifest: &ProjectManifestV1,
    parent_scene_id: SceneId,
    member_id: SceneMemberId,
    layer_offset: sdf::LayerOffset,
) -> Result<Vec<SceneId>> {
    ensure!(
        layer_offset.offset.is_finite(),
        "member reference layer offset must be finite"
    );
    ensure!(
        layer_offset.scale.is_finite() && layer_offset.scale != 0.0,
        "member reference layer scale must be finite and non-zero"
    );

    let mut staged_overrides = TimelineStageOverrides::default();
    staged_overrides.populate_manifest_targets(project_root, manifest);
    let parent_path = staged_overrides
        .scene_targets
        .get(&parent_scene_id)
        .cloned()
        .unwrap_or_else(|| scene_path(project_root, parent_scene_id));
    let members = read_scene_members(&parent_path, parent_scene_id)?;
    ensure!(
        members.iter().any(|member| member.id == member_id),
        "Scene member {member_id} is not authored in parent Scene {parent_scene_id}"
    );

    let stage = Stage::open(parent_path.to_string_lossy().as_ref())
        .context("open parent Scene for member retiming")?;
    let member_path = scene_member_path_for_stage(&stage, member_id)?;
    let spec_path = sdf::path(member_path.as_str())?;
    let Some(Value::ReferenceListOp(references)) = stage
        .root_layer()
        .prim(spec_path)?
        .map(|spec| spec.field(REFERENCES_FIELD))
        .transpose()?
        .flatten()
    else {
        bail!("Scene member {member_id} is missing its target reference");
    };
    let updated_refs: Vec<sdf::Reference> = references
        .iter()
        .cloned()
        .map(|mut reference| {
            reference.layer_offset = layer_offset;
            reference
        })
        .collect();
    ensure!(
        !updated_refs.is_empty(),
        "Scene member {member_id} has an empty reference list"
    );
    stage.prim(member_path.as_str())?.set_metadata(
        REFERENCES_FIELD,
        Value::ReferenceListOp(sdf::ReferenceListOp::prepended(updated_refs)),
    )?;
    recompute_scene_stage_timeline(
        &stage,
        project_root,
        &parent_path,
        &members,
        &staged_overrides,
    )?;

    let transaction_dir = project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join(TRANSACTIONS_DIRECTORY)
        .join(format!("retime-{}", Uuid::new_v4()));
    fs::create_dir_all(&transaction_dir)
        .context("create member retiming transaction directory")?;

    let mut staged_updates: Vec<(SceneId, PathBuf, PathBuf)> = Vec::new();
    let prepare_result = (|| -> Result<()> {
        let temp_parent = transaction_dir.join(format!("{parent_scene_id}.tmp.usda"));
        stage
            .root_layer()
            .export(temp_parent.to_string_lossy().as_ref())
            .context("export retimed parent Scene")?;
        validate_scene_file(&temp_parent, parent_scene_id, &members)?;
        staged_overrides
            .scenes
            .insert(parent_scene_id, temp_parent.clone());
        staged_updates.push((parent_scene_id, temp_parent, parent_path));

        let scene_members = read_all_scene_members(project_root, manifest)?;
        all_scenes_child_first(&scene_members, manifest)?;
        let ancestors = affected_ancestors_child_first(&scene_members, &[parent_scene_id])?;
        stage_ancestor_updates(
            project_root,
            &scene_members,
            &ancestors,
            &transaction_dir,
            &mut staged_overrides,
            &mut staged_updates,
        )?;
        Ok(())
    })();

    if let Err(error) = prepare_result {
        let _ = fs::remove_dir_all(&transaction_dir);
        return Err(error);
    }

    publish_staged_layers_atomic(&transaction_dir, &staged_updates)?;
    let _ = fs::remove_dir_all(&transaction_dir);
    Ok(staged_updates
        .into_iter()
        .map(|(scene_id, _, _)| scene_id)
        .collect())
}

pub(super) fn read_all_scene_members(
    project_root: &Path,
    manifest: &ProjectManifestV1,
) -> Result<HashMap<SceneId, Vec<SceneMember>>> {
    read_all_scene_members_with_overrides(
        project_root,
        manifest,
        &TimelineStageOverrides::default(),
    )
}

pub(super) fn read_all_scene_members_with_overrides(
    project_root: &Path,
    manifest: &ProjectManifestV1,
    overrides: &TimelineStageOverrides,
) -> Result<HashMap<SceneId, Vec<SceneMember>>> {
    let layout = ProjectStorageLayout::new(project_root);
    let mut map = HashMap::with_capacity(manifest.scenes.len());
    for scene in &manifest.scenes {
        let path = overrides
            .scenes
            .get(&scene.id)
            .cloned()
            .unwrap_or_else(|| layout.readable_scene_path(manifest, scene));
        ensure!(
            path.is_file(),
            "required Scene layer {} is missing: {}",
            scene.id,
            path.display()
        );
        let members = read_scene_members(&path, scene.id)?;
        map.insert(scene.id, members);
    }
    Ok(map)
}

pub(super) fn all_scenes_child_first(
    scene_members: &HashMap<SceneId, Vec<SceneMember>>,
    manifest: &ProjectManifestV1,
) -> Result<Vec<SceneId>> {
    let all_ids: HashSet<SceneId> = manifest.scenes.iter().map(|s| s.id).collect();
    let model_ids: HashSet<ModelId> = manifest.models.iter().map(|m| m.id).collect();
    for members in scene_members.values() {
        for member in members {
            ensure!(
                match member.target {
                    SceneMemberTarget::Scene(id) => all_ids.contains(&id),
                    SceneMemberTarget::Model(id) => model_ids.contains(&id),
                },
                "unknown managed target for member {}",
                member.id
            );
        }
    }
    let mut in_degree: HashMap<SceneId, usize> = all_ids.iter().map(|&id| (id, 0)).collect();
    let mut dependents: HashMap<SceneId, Vec<SceneId>> = HashMap::new();

    for (&parent_id, members) in scene_members {
        if !all_ids.contains(&parent_id) {
            continue;
        }
        for member in members {
            if let SceneMemberTarget::Scene(child_id) = member.target
                && all_ids.contains(&child_id)
            {
                dependents.entry(child_id).or_default().push(parent_id);
                *in_degree.entry(parent_id).or_insert(0) += 1;
            }
        }
    }

    let mut ready: Vec<SceneId> = in_degree
        .iter()
        .filter_map(|(&id, &deg)| (deg == 0).then_some(id))
        .collect();
    ready.sort();
    let mut queue = VecDeque::from(ready);
    let mut ordered = Vec::with_capacity(all_ids.len());

    while let Some(scene_id) = queue.pop_front() {
        ordered.push(scene_id);
        let mut next = Vec::new();
        for &parent_id in dependents.get(&scene_id).into_iter().flatten() {
            if let Some(deg) = in_degree.get_mut(&parent_id) {
                *deg -= 1;
                if *deg == 0 {
                    next.push(parent_id);
                }
            }
        }
        next.sort();
        queue.extend(next);
    }

    ensure!(
        ordered.len() == all_ids.len(),
        "Project Scene composition cycle detected during timeline inspection"
    );
    Ok(ordered)
}
