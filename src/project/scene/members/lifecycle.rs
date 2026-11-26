use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use openusd::usd::Stage;
use usd_project::{SceneId, SceneMember};

use super::super::authoring::{
    author_scene_member_at_path, prepare_scene_for_direct_members, read_scene_members,
    scene_member_path, sync_parent_best_effort, validate_scene_file,
};

/// Replace all authored placements in one Scene with the supplied set.
pub(crate) fn replace_scene_members_atomic(
    path: &Path,
    project_root: &Path,
    expected_scene_id: SceneId,
    members: &[SceneMember],
) -> Result<()> {
    let temporary_path = path.with_file_name(format!(
        ".{}.replace-{}.tmp.usda", expected_scene_id, uuid::Uuid::new_v4(),
    ));
    let result = (|| {
        prepare_scene_members_replacement(path, &temporary_path, project_root, expected_scene_id,
            members, &crate::project::scene::timeline::TimelineStageOverrides::default())?;
        fs::rename(&temporary_path, path).context("publish Scene after placement replacement")?;
        sync_parent_best_effort(path.parent());
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

/// Stage replacement of authored members without publishing any canonical layer.
pub(crate) fn prepare_scene_members_replacement(
    path: &Path,
    temporary_path: &Path,
    project_root: &Path,
    expected_scene_id: SceneId,
    members: &[SceneMember],
    overrides: &crate::project::scene::timeline::TimelineStageOverrides,
) -> Result<()> {
    let existing = read_scene_members(path, expected_scene_id)?;
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::open(&path_string).context("open Scene for placement replacement")?;
    prepare_scene_for_direct_members(&stage)?;
    let mut preserved_offsets = std::collections::HashMap::new();
    for member in &existing {
        let offset = crate::project::scene::timeline::read_member_reference_offset(&stage, member.id)?;
        if !offset.is_identity() {
            preserved_offsets.insert(member.id, offset);
        }
    }
    for member in existing {
        if stage
            .prim(scene_member_path(member.id).as_str())?
            .is_defined()?
        {
            ensure!(
                stage.remove_prim(scene_member_path(member.id).as_str())?,
                "Project Scene placement was not authored in the parent layer"
            );
        }
    }
    for member in members {
        author_scene_member_at_path(&stage, project_root, path, member, None)?;
        if let Some(offset) = preserved_offsets.get(&member.id).copied() {
            let member_path = scene_member_path(member.id);
            let spec_path = openusd::sdf::path(member_path.as_str())?;
            if let Some(openusd::vt::Value::ReferenceListOp(references)) = stage
                .root_layer()
                .prim(spec_path)?
                .map(|spec| spec.field(super::super::authoring::REFERENCES_FIELD))
                .transpose()?
                .flatten()
            {
                let updated: Vec<openusd::sdf::Reference> = references
                    .iter()
                    .cloned()
                    .map(|mut reference| {
                        reference.layer_offset = offset;
                        reference
                    })
                    .collect();
                stage.prim(member_path.as_str())?.set_metadata(
                    super::super::authoring::REFERENCES_FIELD,
                    openusd::vt::Value::ReferenceListOp(openusd::sdf::ReferenceListOp::prepended(
                        updated,
                    )),
                )?;
            }
        }
    }
    crate::project::scene::timeline::recompute_scene_stage_timeline(
        &stage,
        project_root,
        path,
        members,
        overrides,
    )?;
    stage.root_layer().export(temporary_path.to_string_lossy().as_ref())
        .context("export Scene after placement replacement")?;
    validate_scene_file(temporary_path, expected_scene_id, members)?;
    Ok(())
}
