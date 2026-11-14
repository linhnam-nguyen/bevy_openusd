//! Scene rename/delete/recreate steps for the OR8 M2 lifecycle matrix.

use project_protocol::{
    ProjectDeleteSceneRequest, ProjectWriteError, ProjectWriteErrorCode, ProjectWriteTarget,
};
use usd_project::SceneId;

use crate::project::catalog::manifest_store::ManifestStore;

use super::matrix::Context;

pub(super) fn rename_delete_recreate(context: &mut Context) -> Result<(), String> {
    let selected = [
        context.fixture.identity("Sc1.1").id,
        context.fixture.identities_named("Sc1.1")[1].id,
    ][context.rng.choose_index(2)];
    let selected_parent = context
        .fixture
        .scenes
        .iter()
        .find(|scene| scene.id == selected)
        .and_then(|scene| scene.parent)
        .ok_or_else(|| {
            context
                .trace
                .failure("selected lifecycle Scene has no parent")
        })?;
    let other_parent = [
        context.fixture.identity("Sc1").id,
        context.fixture.identity("Sc2").id,
    ]
    .into_iter()
    .find(|parent| *parent != selected_parent)
    .ok_or_else(|| context.trace.failure("no alternate lifecycle parent"))?;
    context
        .trace
        .decision(format!("lifecycle_selected_scene={selected}"));
    let storage_key = ManifestStore::read_validated(&context.project_root)
        .map_err(|error| {
            context
                .trace
                .failure(format!("read lifecycle manifest: {error}"))
        })?
        .scene(selected)
        .ok_or_else(|| context.trace.failure("selected lifecycle Scene is absent"))?
        .storage_key
        .as_str()
        .to_owned();
    let fresh = format!("M2C8Fresh{:x}", context.rng.next_u64());
    context
        .trace
        .operation(format!("rename scene={selected} name={fresh}"));
    context
        .service
        .rename(
            context.fixture.project.id,
            ProjectWriteTarget::Scene(selected),
            &fresh,
        )
        .map_err(|error| context.trace.failure(format!("fresh rename: {error}")))?;
    assert_name(context, selected, &fresh)?;
    context
        .trace
        .operation(format!("rename scene={selected} duplicate_name=Sc1.2"));
    context
        .service
        .rename(
            context.fixture.project.id,
            ProjectWriteTarget::Scene(selected),
            "Sc1.2",
        )
        .map_err(|error| context.trace.failure(format!("duplicate rename: {error}")))?;
    assert_name(context, selected, "Sc1.2")?;
    context.trace.operation(format!("delete scene={selected}"));
    context
        .service
        .delete_scene(ProjectDeleteSceneRequest {
            project_id: context.fixture.project.id,
            scene_id: selected,
        })
        .map_err(|error| context.trace.failure(format!("delete Scene: {error}")))?;
    let recreated = context
        .service
        .create_scene(
            context.fixture.project.id,
            ProjectWriteTarget::Scene(other_parent),
            &storage_key,
        )
        .map_err(|error| context.trace.failure(format!("recreate Scene: {error}")))?;
    if recreated.scene_id == selected {
        return Err(context
            .trace
            .failure("recreated Scene reused deleted identity"));
    }
    context.trace.operation(format!(
        "recreate parent={other_parent} storage_key={storage_key} new_scene={}",
        recreated.scene_id
    ));
    context
        .service
        .rename(
            context.fixture.project.id,
            ProjectWriteTarget::Scene(recreated.scene_id),
            "Sc1.2",
        )
        .map_err(|error| {
            context
                .trace
                .failure(format!("recreated duplicate rename: {error}"))
        })?;
    assert_name(context, recreated.scene_id, "Sc1.2")?;
    let protected = context.service.delete_scene(ProjectDeleteSceneRequest {
        project_id: context.fixture.project.id,
        scene_id: context.fixture.root_scene_id,
    });
    if protected
        != Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::ProtectedRootScene,
        })
    {
        return Err(context
            .trace
            .failure("protected root deletion was not rejected"));
    }
    Ok(())
}

fn assert_name(context: &Context, scene_id: SceneId, expected: &str) -> Result<(), String> {
    let actual = ManifestStore::read_validated(&context.project_root)
        .map_err(|error| {
            context
                .trace
                .failure(format!("read renamed manifest: {error}"))
        })?
        .scene(scene_id)
        .ok_or_else(|| {
            context
                .trace
                .failure(format!("Scene {scene_id} missing after rename"))
        })?
        .display_name
        .clone();
    if actual != expected {
        return Err(context
            .trace
            .failure(format!("Scene {scene_id} name {actual:?} != {expected:?}")));
    }
    Ok(())
}
