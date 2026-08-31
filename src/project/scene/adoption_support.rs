use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use openusd::usd::{InitialLoadSet, Stage};
use usd_project::{
    CompositionClassification, CompositionInspection, ProjectManifestV1, SceneCompositionGraph,
    SceneId, SceneMember, SceneMemberId, SceneMemberTarget, ScenePlacementTransform,
};

use super::inspection::inspect_composition;
use crate::project::catalog::manifest_store::ManifestStore;

pub(super) fn ensure_current_manifest(
    project_root: &Path,
    expected: &ProjectManifestV1,
) -> Result<()> {
    let current = ManifestStore::read_validated(project_root)
        .context("read current Project manifest before Scene adoption")?;
    ensure!(
        current.raw() == &expected.canonicalized(),
        "Project manifest changed after the adoption candidate was inspected"
    );
    Ok(())
}

pub(super) fn propose_scene_placement_with_name(
    graph: &SceneCompositionGraph,
    parent_scene_id: SceneId,
    target_scene_id: SceneId,
    name: &str,
    transform: ScenePlacementTransform,
) -> Result<(SceneCompositionGraph, SceneMember)> {
    let mut proposed_graph = graph.clone();
    proposed_graph
        .add_placement(parent_scene_id, target_scene_id)
        .context("validate proposed Scene placement")?;
    Ok((
        proposed_graph,
        SceneMember {
            id: SceneMemberId::new_v4(),
            target: SceneMemberTarget::Scene(target_scene_id),
            name: (!name.is_empty()).then(|| name.to_owned()),
            transform,
        },
    ))
}

pub(crate) fn ensure_adoptable(inspection: &CompositionInspection) -> Result<()> {
    ensure!(
        matches!(
            inspection.classification,
            CompositionClassification::NativeUsdHubScene | CompositionClassification::SceneLike
        ),
        "USD source is not an eligible Scene adoption candidate"
    );
    ensure!(
        inspection.dependencies.iter().all(|dependency| {
            !matches!(
                dependency.classification,
                usd_project::DependencyClassification::Missing
                    | usd_project::DependencyClassification::Unsupported
            )
        }),
        "USD source has unresolved or unsupported composition dependencies"
    );
    Ok(())
}

pub(crate) fn revalidate_source(source: &Path, expected: &CompositionInspection) -> Result<String> {
    ensure!(
        source.is_file(),
        "Scene adoption source disappeared or is not a file"
    );
    let actual = inspect_composition(source).context("reinspect Scene adoption source")?;
    ensure!(
        &actual == expected,
        "Scene adoption source changed after inspection"
    );
    ensure_adoptable(&actual)?;

    let source_string = source
        .to_str()
        .context("Scene adoption source path must be valid UTF-8")?;
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(source_string)
        .context("reopen Scene adoption source")?;
    stage
        .default_prim()
        .map(|token| token.as_str().to_owned())
        .context("Scene adoption source has no defaultPrim")
}

pub(super) fn rollback_publication(
    parent_path: Option<&Path>,
    backup_path: &Path,
    backup_created: bool,
    parent_published: bool,
    scene_path: &Path,
    scene_published: bool,
    source_path: &Path,
    source_published: bool,
    binding_path: &Path,
    binding_published: bool,
) -> Result<()> {
    if scene_published {
        fs::remove_file(scene_path).context("remove newly published Scene layer")?;
    }
    if source_published {
        fs::remove_dir_all(source_path).context("remove newly published Scene source closure")?;
    }
    if binding_published {
        fs::remove_file(binding_path).context("remove newly published linked source binding")?;
    }
    if parent_published {
        let parent_path = parent_path.context("published parent Scene path is missing")?;
        if backup_created {
            fs::remove_file(parent_path).context("remove replaced parent Scene layer")?;
            fs::rename(backup_path, parent_path).context("restore parent Scene layer backup")?;
        } else {
            fs::remove_file(parent_path).context("remove newly published parent Scene layer")?;
        }
    }
    Ok(())
}
