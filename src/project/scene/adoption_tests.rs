use std::{fs, path::Path};

use anyhow::Result;
use openusd::{sdf::Value, usd::Stage};
use tempfile::tempdir;
use usd_project::{
    ProjectId, ProjectManifestV1, ProjectRoot, SceneCompositionGraph, SceneId, SceneManifestEntry,
    SceneMemberTarget, ScenePlacementTransform, StorageKey,
};

use super::*;
use crate::project::{
    catalog::manifest_store::ManifestStore,
    scene::{authoring::author_scene_atomic, inspection::inspect_composition},
};

fn scene_manifest(scene_id: SceneId) -> SceneManifestEntry {
    SceneManifestEntry {
        id: scene_id,
        storage_key: StorageKey::new(scene_id.to_string()).unwrap(),
        display_name: scene_id.to_string(),
    }
}

fn manifest(
    project_id: ProjectId,
    scenes: Vec<SceneManifestEntry>,
    root: ProjectRoot,
) -> ProjectManifestV1 {
    ProjectManifestV1::new(project_id, "Adoption Project", root, scenes, vec![])
}

fn write_scene_candidate(directory: &Path, name: &str) -> std::path::PathBuf {
    let path = directory.join(name);
    fs::write(
        &path,
        r#"#usda 1.0
(
    defaultPrim = "Assembly"
)
def Xform "Assembly" (
    kind = "assembly"
) {
    def Xform "Child" {}
}
"#,
    )
    .unwrap();
    path
}

fn request<'a>(
    project_root: &'a Path,
    source: &'a Path,
    inspection: &'a CompositionInspection,
    base_manifest: &'a ProjectManifestV1,
    graph: &'a SceneCompositionGraph,
) -> SceneAdoptionRequest<'a> {
    SceneAdoptionRequest {
        project_root,
        source,
        inspection,
        name: "Imported Scene",
        base_manifest,
        graph,
        parent_scene_id: None,
        parent_members: &[],
        target_scene_id: None,
        set_as_root: true,
        placement: ScenePlacementTransform::IDENTITY,
    }
}

#[test]
fn empty_project_candidate_can_become_root() -> Result<()> {
    let project = tempdir()?;
    let source = write_scene_candidate(project.path(), "source.usda");
    let base = manifest(ProjectId::new_v4(), vec![], ProjectRoot::Empty);
    ManifestStore::write_manifest_atomic(project.path(), &base)?;
    let inspection = inspect_composition(&source)?;

    let adopted = adopt_scene_atomic(request(
        project.path(),
        &source,
        &inspection,
        &base,
        &SceneCompositionGraph::default(),
    ))?;

    assert_eq!(
        &adopted.manifest.root,
        &ProjectRoot::Scene(adopted.scene_id)
    );
    assert!(
        adopted
            .manifest
            .scenes
            .iter()
            .any(|entry| entry.id == adopted.scene_id)
    );
    assert_eq!(
        adopted
            .manifest
            .scenes
            .iter()
            .find(|entry| entry.id == adopted.scene_id)
            .expect("adopted Scene is registered")
            .storage_key
            .as_str(),
        "Imported Scene"
    );
    assert!(adopted.scene_path.exists());
    assert_eq!(
        ManifestStore::read_validated(project.path())?.raw(),
        &adopted.manifest.canonicalized()
    );
    Ok(())
}

#[test]
fn nested_adoption_publishes_one_distinct_parent_placement() -> Result<()> {
    let project = tempdir()?;
    let source = write_scene_candidate(project.path(), "nested-source.usda");
    let parent_id = SceneId::new_v4();
    author_scene_atomic(project.path(), parent_id)?;
    let base = manifest(
        ProjectId::new_v4(),
        vec![scene_manifest(parent_id)],
        ProjectRoot::Scene(parent_id),
    );
    ManifestStore::write_manifest_atomic(project.path(), &base)?;
    let inspection = inspect_composition(&source)?;
    let graph = SceneCompositionGraph::default();
    let mut nested_request = request(project.path(), &source, &inspection, &base, &graph);
    nested_request.parent_scene_id = Some(parent_id);

    let adopted = adopt_scene_atomic(nested_request)?;
    let member = adopted
        .member
        .expect("nested adoption should return a placement");
    assert_eq!(member.target, SceneMemberTarget::Scene(adopted.scene_id));
    assert_eq!(member.name.as_deref(), Some("Imported Scene"));
    let parent_path = crate::project::scene::authoring::scene_path(project.path(), parent_id);
    let parent_stage = Stage::open(&parent_path.to_string_lossy())?;
    let member_path = crate::project::scene::authoring::scene_member_path(member.id);
    let parent_member = parent_stage.prim(member_path.as_str());
    assert!(parent_member.is_defined()?);
    let Some(Value::Dictionary(data)) = parent_member.custom_data()? else {
        panic!("nested Scene placement should retain customData");
    };
    let target_id = adopted.scene_id.to_string();
    assert_eq!(
        data.get("usdhub:targetId").and_then(Value::as_str),
        Some(target_id.as_str())
    );
    Ok(())
}

#[test]
fn repeated_placement_keeps_one_target_identity_and_distinct_members() -> Result<()> {
    let target = SceneId::new_v4();
    let parent = SceneId::new_v4();
    let graph = SceneCompositionGraph::default();
    let (_, first) = propose_scene_placement(&graph, parent, target)?;
    let (_, second) = propose_scene_placement(&graph, parent, target)?;

    assert_ne!(first.id, second.id);
    assert_eq!(first.target, second.target);
    Ok(())
}

#[test]
fn source_disappearance_leaves_manifest_and_scene_catalogue_unchanged() -> Result<()> {
    let project = tempdir()?;
    let source = write_scene_candidate(project.path(), "disappearing.usda");
    let base = manifest(ProjectId::new_v4(), vec![], ProjectRoot::Empty);
    ManifestStore::write_manifest_atomic(project.path(), &base)?;
    let before = fs::read(crate::project::catalog::manifest_store::manifest_path(
        project.path(),
    ))?;
    let inspection = inspect_composition(&source)?;
    fs::remove_file(&source)?;

    assert!(
        adopt_scene_atomic(request(
            project.path(),
            &source,
            &inspection,
            &base,
            &SceneCompositionGraph::default(),
        ))
        .is_err()
    );
    assert_eq!(
        fs::read(crate::project::catalog::manifest_store::manifest_path(
            project.path()
        ))?,
        before
    );
    let scenes_directory = project.path().join(".usdhub/scenes");
    assert!(!scenes_directory.exists() || fs::read_dir(scenes_directory)?.next().is_none());
    Ok(())
}

#[test]
fn cycle_revalidation_leaves_manifest_unchanged() -> Result<()> {
    let project = tempdir()?;
    let source = write_scene_candidate(project.path(), "cycle.usda");
    let ancestor = SceneId::new_v4();
    let child = SceneId::new_v4();
    author_scene_atomic(project.path(), ancestor)?;
    author_scene_atomic(project.path(), child)?;
    let base = manifest(
        ProjectId::new_v4(),
        vec![scene_manifest(ancestor), scene_manifest(child)],
        ProjectRoot::Scene(ancestor),
    );
    ManifestStore::write_manifest_atomic(project.path(), &base)?;
    let before = fs::read(crate::project::catalog::manifest_store::manifest_path(
        project.path(),
    ))?;
    let inspection = inspect_composition(&source)?;
    let graph = SceneCompositionGraph::from_edges([(ancestor, child)]);
    let mut cycle_request = request(project.path(), &source, &inspection, &base, &graph);
    cycle_request.parent_scene_id = Some(child);
    cycle_request.parent_members = &[];
    cycle_request.target_scene_id = Some(ancestor);
    cycle_request.set_as_root = false;

    assert!(adopt_scene_atomic(cycle_request).is_err());
    assert_eq!(
        fs::read(crate::project::catalog::manifest_store::manifest_path(
            project.path()
        ))?,
        before
    );
    Ok(())
}

#[test]
fn dependency_failure_leaves_old_manifest_byte_for_byte_valid() -> Result<()> {
    let project = tempdir()?;
    let source = project.path().join("missing-dependency.usda");
    fs::write(
        &source,
        r#"#usda 1.0
(
    defaultPrim = "Assembly"
)
def Xform "Assembly" (
    kind = "assembly"
    references = @./does-not-exist.usda@</Asset>
) {}
"#,
    )?;
    let base = manifest(ProjectId::new_v4(), vec![], ProjectRoot::Empty);
    ManifestStore::write_manifest_atomic(project.path(), &base)?;
    let before = fs::read(crate::project::catalog::manifest_store::manifest_path(
        project.path(),
    ))?;
    let inspection = inspect_composition(&source)?;

    assert!(
        adopt_scene_atomic(request(
            project.path(),
            &source,
            &inspection,
            &base,
            &SceneCompositionGraph::default(),
        ))
        .is_err()
    );
    assert_eq!(
        fs::read(crate::project::catalog::manifest_store::manifest_path(
            project.path()
        ))?,
        before
    );
    Ok(())
}
