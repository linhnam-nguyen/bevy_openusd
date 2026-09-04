use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use openusd::usd::{InitialLoadSet, Stage};
use thiserror::Error;
use usd_project::{
    CompositionClassification, CompositionInspection, ProjectManifestV1, SceneCompositionGraph,
    SceneId, SceneMember, SceneMemberId, SceneMemberTarget, ScenePlacementTransform,
};

use super::inspection::inspect_composition;
use crate::project::catalog::manifest_store::ManifestStore;

#[derive(Debug, Error)]
pub(crate) enum SourceRevalidationError {
    #[error("Scene adoption source is missing or changed")]
    Changed,
    #[error("Scene adoption source composition could not be validated: {0}")]
    CompositionValidation(#[source] anyhow::Error),
    #[error("Scene adoption source classification or dependency is rejected: {0}")]
    ClassificationRejected(#[source] anyhow::Error),
}

/// Stable internal adoption phases. The service maps these phases to
/// path-free protocol error codes instead of exposing a broad anyhow context
/// or filesystem details to the UI.
#[derive(Debug, Error)]
pub(crate) enum AdoptionPhaseError {
    #[error("Scene adoption source classification was rejected: {0}")]
    ClassificationRejected(#[source] anyhow::Error),
    #[error("Scene adoption dependency localization failed: {0}")]
    DependencyLocalization(#[source] anyhow::Error),
    #[error("Scene adoption composition validation failed: {0}")]
    CompositionValidation(#[source] anyhow::Error),
    #[error("Scene adoption publication failed: {0}")]
    Publication(#[source] anyhow::Error),
}

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
        !matches!(
            inspection.classification,
            CompositionClassification::Unsupported
        ),
        "USD source cannot be safely inspected for Scene adoption"
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

pub(crate) fn revalidate_source(
    source: &Path,
    expected: &CompositionInspection,
) -> Result<Vec<String>> {
    revalidate_source_for_adoption(source, expected).map_err(anyhow::Error::new)
}

pub(crate) fn revalidate_source_for_adoption(
    source: &Path,
    expected: &CompositionInspection,
) -> std::result::Result<Vec<String>, SourceRevalidationError> {
    if !source.is_file() {
        return Err(SourceRevalidationError::Changed);
    }
    let actual =
        inspect_composition(source).map_err(SourceRevalidationError::CompositionValidation)?;
    if &actual != expected {
        return Err(SourceRevalidationError::Changed);
    }
    ensure_adoptable(&actual).map_err(SourceRevalidationError::ClassificationRejected)?;

    let source_string = source.to_str().ok_or_else(|| {
        SourceRevalidationError::CompositionValidation(anyhow::anyhow!(
            "Scene adoption source path must be valid UTF-8"
        ))
    })?;
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(source_string)
        .map_err(|error| {
            SourceRevalidationError::CompositionValidation(anyhow::anyhow!("{error:#}"))
        })?;
    source_entrypoint_prims(&stage).map_err(SourceRevalidationError::CompositionValidation)
}

/// Return the explicit root references used by Project wrappers.  A
/// defaultPrim remains the preferred single entry point; otherwise every
/// defined top-level prim is preserved as a separate reference target.
pub(crate) fn source_entrypoint_prims(stage: &Stage) -> Result<Vec<String>> {
    if let Some(default_prim) = stage.default_prim() {
        let path = format!("/{default_prim}");
        ensure!(
            stage.prim(path.as_str()).is_defined()?,
            "USD source defaultPrim is not a defined prim"
        );
        return Ok(vec![path]);
    }

    let mut roots = Vec::new();
    for prim in stage.prim("/").children()? {
        if prim.is_active()? && prim.is_defined()? && !prim.is_abstract()? {
            roots.push(prim.path().as_str().to_owned());
        }
    }
    roots.sort();
    ensure!(
        !roots.is_empty(),
        "USD source has no defaultPrim and no defined root prims"
    );
    Ok(roots)
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
