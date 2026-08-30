use super::*;
use crate::project::scene::adoption_authoring::author_scene_wrapper_to_path;
use crate::project::spatial::inspect_source;
use tempfile::tempdir;

#[test]
fn status_distinguishes_sync_change_and_source_removal() {
    let project = tempdir().unwrap();
    let source = project.path().join("source.usda");
    fs::write(&source, b"#usda 1.0\n").unwrap();
    let temporary = project.path().join("binding.tmp");
    let scene_id = SceneId::new_v4();
    prepare_binding(&temporary, scene_id, &source).unwrap();
    fs::create_dir_all(ProjectStorageLayout::new(project.path()).links_dir()).unwrap();
    fs::rename(&temporary, binding_path(project.path(), scene_id)).unwrap();
    assert_eq!(
        status(project.path(), scene_id).unwrap(),
        LinkedSourceStatus::InSync
    );

    fs::write(&source, b"#usda 1.0\n# changed\n").unwrap();
    assert_eq!(
        status(project.path(), scene_id).unwrap(),
        LinkedSourceStatus::OutOfSync
    );

    fs::remove_file(&source).unwrap();
    assert_eq!(
        status(project.path(), scene_id).unwrap(),
        LinkedSourceStatus::SourceUnavailable
    );
}

#[test]
fn status_detects_dependency_closure_change() {
    let project = tempdir().unwrap();
    let dependency = project.path().join("dependency.usda");
    fs::write(&dependency, "#usda 1.0\ndef Xform \"Asset\" {}\n").unwrap();
    let source = project.path().join("assembly.usda");
    fs::write(
        &source,
        "#usda 1.0\ndef Xform \"Assembly\" (references = @./dependency.usda@</Asset>) {}\n",
    )
    .unwrap();
    let scene_id = SceneId::new_v4();
    let binding_directory = tempdir().unwrap();
    let temporary = binding_directory.path().join("binding.tmp");
    prepare_binding(&temporary, scene_id, &source).unwrap();
    fs::create_dir_all(ProjectStorageLayout::new(project.path()).links_dir()).unwrap();
    fs::rename(&temporary, binding_path(project.path(), scene_id)).unwrap();
    assert_eq!(
        status(project.path(), scene_id).unwrap(),
        LinkedSourceStatus::InSync
    );

    fs::write(
        &dependency,
        "#usda 1.0\ndef Xform \"Asset\" { int changed = 1 }\n",
    )
    .unwrap();
    assert_eq!(
        status(project.path(), scene_id).unwrap(),
        LinkedSourceStatus::OutOfSync
    );
}

#[test]
fn missing_binding_is_unavailable_only_for_linked_scene_wrappers() {
    let project = tempdir().unwrap();
    let source = project.path().join("source.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" {}\n",
    )
    .unwrap();
    let spatial = inspect_source(&source).unwrap();
    let linked_id = SceneId::new_v4();
    let linked_path = project.path().join("linked.usda");
    author_scene_wrapper_to_path(
        &linked_path,
        project.path(),
        &linked_path,
        linked_id,
        &source,
        "Assembly",
        "Linked",
        &spatial,
        true,
    )
    .unwrap();
    assert_eq!(
        status_for_scene(project.path(), &linked_path, linked_id).unwrap(),
        Some(LinkedSourceStatus::SourceUnavailable)
    );

    let imported_id = SceneId::new_v4();
    let imported_path = project.path().join("imported.usda");
    author_scene_wrapper_to_path(
        &imported_path,
        project.path(),
        &imported_path,
        imported_id,
        &source,
        "Assembly",
        "Imported",
        &spatial,
        false,
    )
    .unwrap();
    assert_eq!(
        status_for_scene(project.path(), &imported_path, imported_id).unwrap(),
        None
    );
}

#[test]
fn binding_status_does_not_open_the_scene_wrapper() {
    let project = tempdir().unwrap();
    let source = project.path().join("source.usda");
    fs::write(&source, b"#usda 1.0\n").unwrap();
    let scene_id = SceneId::new_v4();
    let temporary = project.path().join("binding.tmp");
    prepare_binding(&temporary, scene_id, &source).unwrap();
    fs::create_dir_all(ProjectStorageLayout::new(project.path()).links_dir()).unwrap();
    fs::rename(&temporary, binding_path(project.path(), scene_id)).unwrap();

    assert_eq!(
        status_for_scene(
            project.path(),
            &project.path().join("missing-scene.usda"),
            scene_id,
        )
        .unwrap(),
        Some(LinkedSourceStatus::InSync)
    );
}

#[test]
fn legacy_linked_wrapper_is_marked_before_binding_is_removed() {
    let (project, manifest, scene_id, scene_path, source) = legacy_wrapper_fixture();
    write_binding(project.path(), scene_id, &source);
    fs::remove_file(&source).unwrap();

    migrate_linked_source_provenance(project.path(), &manifest).unwrap();
    fs::remove_file(binding_path(project.path(), scene_id)).unwrap();

    assert_eq!(
        status_for_scene(project.path(), &scene_path, scene_id).unwrap(),
        Some(LinkedSourceStatus::SourceUnavailable)
    );
}

#[test]
fn invalid_binding_evidence_does_not_mark_legacy_wrapper() {
    let (project, manifest, scene_id, scene_path, source) = legacy_wrapper_fixture();
    let mismatched_id = SceneId::new_v4();
    let invalid_bindings = [
        ("malformed", b"{".to_vec()),
        (
            "wrong scene id",
            serde_json::to_vec(&LinkedSourceBinding {
                schema_version: BINDING_SCHEMA_VERSION,
                scene_id: mismatched_id,
                source_path: source.clone(),
                source_fingerprint: "unused".to_owned(),
            })
            .unwrap(),
        ),
        (
            "unsupported schema",
            serde_json::to_vec(&LinkedSourceBinding {
                schema_version: BINDING_SCHEMA_VERSION + 1,
                scene_id,
                source_path: source.clone(),
                source_fingerprint: "unused".to_owned(),
            })
            .unwrap(),
        ),
    ];

    fs::create_dir_all(ProjectStorageLayout::new(project.path()).links_dir()).unwrap();
    for (label, bytes) in invalid_bindings {
        fs::write(binding_path(project.path(), scene_id), bytes).unwrap();
        migrate_linked_source_provenance(project.path(), &manifest).unwrap();

        let stage = Stage::builder()
            .load(InitialLoadSet::LoadNone)
            .open(scene_path.to_string_lossy().as_ref())
            .unwrap();
        assert_eq!(
            crate::project::spatial::source_binding_marker(&stage.prim(SCENE_SOURCE_PRIM)).unwrap(),
            None,
            "invalid {label} binding must not backfill provenance"
        );
    }
}

fn legacy_wrapper_fixture() -> (
    tempfile::TempDir,
    ProjectManifestV1,
    SceneId,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let project = tempdir().unwrap();
    let source = project.path().join("source.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" {}\n",
    )
    .unwrap();
    let spatial = inspect_source(&source).unwrap();
    let scene_id = SceneId::new_v4();
    let scene = usd_project::SceneManifestEntry {
        id: scene_id,
        storage_key: usd_project::StorageKey::new("Linked").unwrap(),
        display_name: "Linked".to_owned(),
    };
    let manifest = ProjectManifestV1::new(
        usd_project::ProjectId::new_v4(),
        "Project",
        usd_project::ProjectRoot::Empty,
        vec![scene],
        Vec::new(),
    );
    let scene_path = ProjectStorageLayout::new(project.path())
        .canonical_scene_path(&usd_project::StorageKey::new("Linked").unwrap());
    fs::create_dir_all(scene_path.parent().unwrap()).unwrap();
    author_scene_wrapper_to_path(
        &scene_path,
        project.path(),
        &scene_path,
        scene_id,
        &source,
        "Assembly",
        "Linked",
        &spatial,
        false,
    )
    .unwrap();
    (project, manifest, scene_id, scene_path, source)
}

fn write_binding(project_root: &std::path::Path, scene_id: SceneId, source: &std::path::Path) {
    let temporary = project_root.join("binding.tmp");
    prepare_binding(&temporary, scene_id, source).unwrap();
    fs::create_dir_all(ProjectStorageLayout::new(project_root).links_dir()).unwrap();
    fs::rename(&temporary, binding_path(project_root, scene_id)).unwrap();
}
