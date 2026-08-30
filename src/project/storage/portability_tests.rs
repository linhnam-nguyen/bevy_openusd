use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

use anyhow::Context;
use bevy::prelude::App;
use openusd::{sdf, usd::Stage};
use project_protocol::{
    PlacementSpec, ProjectReadCommand, ProjectReadRequest, ProjectReadResponse, ProjectWriteTarget,
};
use tempfile::tempdir;
use usd_model::SnapshotSource;
use usd_project::ProjectContentNode;

use super::super::catalog::manifest_store::ManifestStore;
use super::super::service::ProjectApplicationService;
use super::super::source_closure::dependency_containment_report;
use super::ProjectStorageLayout;

#[test]
fn moved_project_reopens_after_source_and_local_state_removal() -> anyhow::Result<()> {
    let workspace = tempdir()?;
    let path_a = workspace.path().join("A");
    let path_b = workspace.path().join("B");
    fs::create_dir_all(&path_a)?;
    fs::create_dir_all(&path_b)?;
    let external = workspace.path().join("external");
    fs::create_dir_all(&external)?;
    fs::write(
        external.join("dependency.usda"),
        "#usda 1.0\ndef Xform \"Asset\" {}\n",
    )?;
    let source = external.join("assembly.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (kind = \"assembly\" references = @./dependency.usda@</Asset>) {}\n",
    )?;

    let registry_a = workspace.path().join("registry-a.json");
    let mut service =
        ProjectApplicationService::open(&registry_a).context("open source service")?;
    let project = service
        .create_project(&path_a, "Portable")
        .context("create Project at path A")?;
    let project_a = path_a.join("Portable");
    let inspection = crate::project::scene::inspection::inspect_composition(&source)?;
    let linked = service
        .link_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            &source,
            &inspection,
            "Linked Scene".to_owned(),
            "portable-link".to_owned(),
            1,
            PlacementSpec::Default,
        )
        .context("link external Scene at path A")?;
    let manifest = ManifestStore::read_validated(&project_a)?;
    let root_scene = manifest
        .raw()
        .scenes
        .iter()
        .find(|scene| manifest.raw().root == usd_project::ProjectRoot::Scene(scene.id))
        .expect("protected root Scene");
    let root_a =
        ProjectStorageLayout::new(&project_a).canonical_root_scene_path(&root_scene.storage_key);
    let report = dependency_containment_report(&project_a, &root_a)?;
    assert!(report.unresolved.is_empty());
    drop(service);

    copy_tree(&project_a, &path_b.join("Portable"))?;
    let project_b = path_b.join("Portable");
    remove_dir_all_after_cache_workers(&project_a)?;
    fs::remove_dir_all(&external)?;
    fs::remove_dir_all(project_b.join(".usdhub"))?;

    let manifest_b = ManifestStore::read_validated(&project_b)?;
    let root_scene_b = manifest_b
        .raw()
        .scenes
        .iter()
        .find(|scene| manifest_b.raw().root == usd_project::ProjectRoot::Scene(scene.id))
        .expect("protected root Scene after move");
    let layout_b = ProjectStorageLayout::new(&project_b);
    let root_b = layout_b.canonical_root_scene_path(&root_scene_b.storage_key);
    let stage = Stage::open(root_b.to_string_lossy().as_ref())?;
    assert!(stage.composition_errors().is_empty());
    let report_b = dependency_containment_report(&project_b, &root_b)?;
    let canonical_project_b = fs::canonicalize(&project_b)?;
    assert!(
        report_b
            .layers
            .iter()
            .all(|path| path.starts_with(&canonical_project_b)),
        "dependency report escaped moved Project: {report_b:?}"
    );

    let live = usd_bevy::LiveStage::new(stage.clone());
    let mut app = App::new();
    let mut prim_entities = usd_bevy::PrimEntities::default();
    usd_bevy::project_stage(app.world_mut(), &live, &mut prim_entities);
    assert!(prim_entities.entity("/SceneRoot").is_some());
    let placement_id = linked.placement_id.expect("linked Scene placement");
    let placement_path = crate::project::scene::authoring::scene_member_path(placement_id);
    assert!(
        prim_entities.entity(&placement_path).is_some(),
        "placement {placement_path} was not projected; paths: {:?}",
        prim_entities
            .iter()
            .map(|(path, _)| path)
            .collect::<Vec<_>>()
    );
    assert!(
        prim_entities
            .iter()
            .any(|(path, _)| path.starts_with(&format!("{placement_path}/")))
    );

    let semantic = usd_semantic::SemanticExtractor::new(usd_semantic::SemanticConfig::default())
        .extract(
            &stage,
            SnapshotSource::GitCommit {
                oid: "portable-moved".to_owned(),
            },
        )?;
    assert!(
        semantic
            .entities
            .values()
            .any(|entity| entity.prim_path == "/SceneRoot")
    );
    assert!(
        semantic
            .entities
            .values()
            .any(|entity| entity.prim_path == placement_path)
    );

    let registry_b = workspace.path().join("registry-b.json");
    let mut moved_service = ProjectApplicationService::open(&registry_b)?;
    let moved_inspection = moved_service
        .inspect_project(&project_b)
        .context("inspect moved Project")?;
    moved_service
        .import_project(&project_b, &moved_inspection)
        .context("import moved Project")?;
    let tree = moved_service.execute(ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(
        project.id,
    )));
    let ProjectReadResponse::ProjectTree { nodes, .. } = tree.result? else {
        anyhow::bail!("moved Project did not return a tree");
    };
    assert!(nodes.iter().any(|node| matches!(
        node,
        ProjectContentNode::Scene { name, .. } if name == "Portable"
    )));
    assert!(nodes.iter().any(|node| matches!(
        node,
        ProjectContentNode::Scene { name, .. } if name == "Linked Scene"
    )));
    assert!(nodes.iter().any(|node| matches!(
        node,
        ProjectContentNode::Scene {
            scene_id,
            link_status: Some(usd_project::ProjectSceneLinkStatus::SourceUnavailable),
            ..
        } if *scene_id == linked.scene_id
    )));
    Ok(())
}

#[test]
fn pro2_storage_and_composition_acceptance() -> anyhow::Result<()> {
    let workspace = tempdir()?;
    let projects = workspace.path().join("projects");
    fs::create_dir_all(&projects)?;
    let external = workspace.path().join("pro2-source");
    fs::create_dir_all(&external)?;
    fs::write(
        external.join("Looks.usda"),
        "#usda 1.0\ndef Xform \"Looks\" {}\n",
    )?;
    let source = external.join("Projet1.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Projet1\"\n)\ndef Xform \"Projet1\" (kind = \"assembly\" references = @./Looks.usda@</Looks>) {}\n",
    )?;

    let registry = workspace.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(&registry)?;
    let project = service.create_project(&projects, "Pro2")?;
    let project_root = projects.join("Pro2");
    let lv1 = service.create_scene(project.id, ProjectWriteTarget::Project(project.id), "Lv1")?;
    let inspection = crate::project::scene::inspection::inspect_composition(&source)?;
    let projet1 = service.link_scene(
        project.id,
        ProjectWriteTarget::Project(project.id),
        &source,
        &inspection,
        "Projet1".to_owned(),
        "pro2-link".to_owned(),
        1,
        PlacementSpec::Default,
    )?;

    let manifest = ManifestStore::read_validated(&project_root)?;
    let root_scene = manifest
        .raw()
        .scenes
        .iter()
        .find(|scene| manifest.raw().root == usd_project::ProjectRoot::Scene(scene.id))
        .expect("Pro2 protected root Scene");
    let lv1_scene = manifest
        .raw()
        .scenes
        .iter()
        .find(|scene| scene.display_name == "Lv1")
        .expect("Lv1 Scene");
    let projet1_scene = manifest
        .raw()
        .scenes
        .iter()
        .find(|scene| scene.display_name == "Projet1")
        .expect("Projet1 Scene");
    let layout = ProjectStorageLayout::new(&project_root);
    let root_path = layout.canonical_root_scene_path(&root_scene.storage_key);

    assert!(layout.canonical_manifest_path().is_file());
    assert!(root_path.is_file());
    assert!(
        layout
            .canonical_scene_path(&lv1_scene.storage_key)
            .is_file()
    );
    assert!(
        layout
            .canonical_scene_path(&projet1_scene.storage_key)
            .is_file()
    );
    let import_dir = layout.canonical_scene_import_dir(projet1_scene.id);
    assert!(import_dir.join("Projet1.usda").is_file());
    assert!(import_dir.join("Looks.usda").is_file());
    assert!(!layout.legacy_manifest_path().exists());
    assert!(!layout.scenes_dir().exists());
    assert!(!layout.metadata_dir().join("imports").exists());

    let root_stage = Stage::open(root_path.to_string_lossy().as_ref())?;
    assert!(root_stage.composition_errors().is_empty());
    assert_eq!(
        display_name(&root_stage, "/SceneRoot")?,
        Some("Pro2".to_owned())
    );
    assert!(!root_stage.prim("/SceneRoot/Members").is_defined()?);
    let lv1_path = crate::project::scene::authoring::scene_member_path(
        lv1.placement_id.expect("Lv1 placement"),
    );
    let projet1_path = crate::project::scene::authoring::scene_member_path(
        projet1.placement_id.expect("Projet1 placement"),
    );
    assert!(root_stage.prim(lv1_path.as_str()).is_defined()?);
    assert!(root_stage.prim(projet1_path.as_str()).is_defined()?);
    assert_eq!(
        display_name(&root_stage, &lv1_path)?,
        Some("Lv1".to_owned())
    );
    assert_eq!(
        display_name(&root_stage, &projet1_path)?,
        Some("Projet1".to_owned())
    );
    assert!(
        root_stage
            .prim(projet1_path.as_str())
            .children()?
            .into_iter()
            .any(|child| child.path().as_str() != projet1_path)
    );

    let report = dependency_containment_report(&project_root, &root_path)?;
    assert!(report.unresolved.is_empty());
    let canonical_project = fs::canonicalize(&project_root)?;
    assert!(
        report
            .layers
            .iter()
            .chain(report.non_layer_assets.iter())
            .all(|path| path.starts_with(&canonical_project))
    );

    let tree = service.execute(ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(
        project.id,
    )));
    let ProjectReadResponse::ProjectTree { nodes, counts, .. } = tree.result? else {
        anyhow::bail!("Pro2 Project tree read failed");
    };
    assert_eq!(counts.scenes, 3);
    assert_eq!(counts.scene_placements, 2);
    let mut scene_names = nodes
        .iter()
        .filter_map(|node| match node {
            ProjectContentNode::Scene { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    scene_names.sort_unstable();
    assert_eq!(scene_names, vec!["Lv1", "Pro2", "Projet1"]);
    assert!(nodes.iter().any(|node| matches!(
        node,
        ProjectContentNode::ScenePlacement { name: Some(name), .. } if name == "Lv1"
    )));
    assert!(nodes.iter().any(|node| matches!(
        node,
        ProjectContentNode::ScenePlacement { name: Some(name), .. } if name == "Projet1"
    )));
    Ok(())
}

fn display_name(stage: &Stage, path: &str) -> anyhow::Result<Option<String>> {
    let layer = stage.root_layer();
    let path = sdf::path(path)?;
    let Some(spec) = layer.prim(&path) else {
        return Ok(None);
    };
    Ok(spec
        .field("ui:displayName")?
        .and_then(|value| value.as_str().map(str::to_owned)))
}

fn remove_dir_all_after_cache_workers(path: &Path) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
                if Instant::now() >= deadline {
                    return Err(error.into());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_tree(&source_path, &destination_path)?;
        } else {
            fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}
