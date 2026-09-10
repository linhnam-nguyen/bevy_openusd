use std::fs;

use anyhow::Result;
use image::RgbaImage;
use tempfile::tempdir;
use usd_project::{ProjectId, ProjectManifestV1, ProjectRoot, SceneId};

use super::preparation::wait_for;
use super::*;
use crate::project::catalog::manifest_store::ManifestStore;
use crate::project::model_import::{ModelImportRequest, ModelImporter, UsdModelImporter};
use crate::project::model_wrapper::{
    ModelPlacement, ModelWrapperRequest, publish_model_wrapper_atomic,
};

#[test]
fn empty_project_is_warmed_without_a_stage_open_failure() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Warm Project",
        ProjectRoot::Empty,
        Vec::new(),
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    let queue = ProjectCacheWarmQueue::default();
    let target = ProjectCacheTarget::ProjectRoot;

    assert!(queue.enqueue(directory.path(), target.clone()));
    let descriptor =
        wait_for(&queue, directory.path(), &target)?.expect("empty Project warm completes");
    assert_eq!(descriptor.state, ProjectCacheState::Empty);
    Ok(())
}
#[test]
fn fresh_project_import_warms_root_scene_and_model_to_ready() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_id = usd_project::SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Warm Project",
        ProjectRoot::Scene(scene_id),
        vec![usd_project::SceneManifestEntry {
            id: scene_id,
            storage_key: usd_project::StorageKey::new("scene").unwrap(),
            display_name: "Scene".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    crate::project::scene::authoring::author_scene_atomic(directory.path(), scene_id)?;

    let texture_path = directory.path().join("diffuse.png");
    RgbaImage::from_pixel(1, 1, image::Rgba([32, 160, 224, 255])).save(&texture_path)?;
    let source = directory.path().join("materials.usda");
    fs::write(
        &source,
        format!(
            r#"#usda 1.0
(
    defaultPrim = "World"
)
def Xform "World"
{{
    def Material "Mat"
    {{
        token outputs:surface.connect = </World/Mat/Surface.outputs:surface>
        def Shader "Surface"
        {{
            uniform token info:id = "UsdPreviewSurface"
            color3f inputs:diffuseColor.connect = </World/Mat/Texture.outputs:rgb>
            float inputs:roughness = 0.5
            token outputs:surface
        }}
        def Shader "Texture"
        {{
            uniform token info:id = "UsdUVTexture"
            asset inputs:file = @{texture_path}@
            token outputs:rgb
        }}
    }}
    def Mesh "Triangle" (
        prepend apiSchemas = ["MaterialBindingAPI"]
    )
    {{
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
        rel material:binding = </World/Mat>
    }}
}}
"#,
            texture_path = texture_path.display(),
        ),
    )?;
    let importer = UsdModelImporter;
    let inspection = importer.inspect(&source)?;
    let prepared = importer.prepare(ModelImportRequest { source, inspection })?;
    let model_id = prepared.id;
    let published = publish_model_wrapper_atomic(ModelWrapperRequest {
        project_root: directory.path(),
        base_manifest: &manifest,
        prepared: &prepared,
        set_as_root: false,
        placement: Some(ModelPlacement {
            parent_scene_id: scene_id,
            parent_members: &[],
            transform: Default::default(),
        }),
    })?;

    let queue = ProjectCacheWarmQueue::default();
    assert!(queue.enqueue_project_targets(directory.path()));
    for target in [
        ProjectCacheTarget::ProjectRoot,
        ProjectCacheTarget::Scene {
            id: scene_id.to_string(),
        },
        ProjectCacheTarget::Model {
            id: model_id.to_string(),
        },
    ] {
        let descriptor = wait_for(&queue, directory.path(), &target)?
            .expect("fresh Project target warm completes");
        assert_eq!(descriptor.state, ProjectCacheState::Ready);
        let runtime = descriptor.runtime.expect("Ready runtime manifest");
        assert!(!runtime.hierarchy.blob_id.is_empty());
        assert!(!runtime.meshes.is_empty(), "mesh payload must be warmed");
        assert!(
            !runtime.materials.is_empty(),
            "material payload must be warmed"
        );
        assert!(
            !runtime.textures.is_empty(),
            "texture payload must be warmed"
        );
    }
    assert_eq!(published.manifest.root, ProjectRoot::Scene(scene_id));
    Ok(())
}

#[test]
fn duplicate_warm_requests_are_coalesced() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Warm Project",
        ProjectRoot::Empty,
        Vec::new(),
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    fs::create_dir_all(directory.path().join(".usdhub/cache"))?;
    let queue = ProjectCacheWarmQueue::default();
    let target = ProjectCacheTarget::ProjectRoot;

    assert!(queue.enqueue(directory.path(), target.clone()));
    assert!(queue.enqueue(directory.path(), target));
    Ok(())
}

#[test]
fn affected_targets_preserve_scene_and_model_owned_boundaries() -> Result<()> {
    let directory = tempdir()?;
    let scene_id = usd_project::SceneId::new_v4();
    let model_id = usd_project::ModelId::new_v4();

    let scene_keys = affected_targets(
        directory.path(),
        &ProjectCacheTarget::Scene { id: scene_id.to_string() },
    )?
    .into_iter()
    .map(|target| target.key())
    .collect::<Vec<_>>();
    assert_eq!(scene_keys, vec![format!("scene:{scene_id}"), "project".to_owned()]);

    let model_keys = affected_targets(
        directory.path(),
        &ProjectCacheTarget::Model { id: model_id.to_string() },
    )?
    .into_iter()
    .map(|target| target.key())
    .collect::<Vec<_>>();
    assert_eq!(model_keys, vec![format!("model:{model_id}"), "project".to_owned()]);
    Ok(())
}

#[test]
fn target_content_identity_changes_with_target_content() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_id = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Warm Project",
        ProjectRoot::Scene(scene_id),
        vec![usd_project::SceneManifestEntry {
            id: scene_id,
            storage_key: usd_project::StorageKey::new("scene").unwrap(),
            display_name: "Scene".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    let scene_path =
        crate::project::scene::authoring::author_scene_atomic(directory.path(), scene_id)?;
    let first = ProjectCacheIdentity::for_project(
        directory.path(),
        ProjectCacheTarget::ProjectRoot,
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;
    let mut changed_scene = fs::read(&scene_path)?;
    changed_scene.extend_from_slice(b"\n# target content changed\n");
    fs::write(&scene_path, changed_scene)?;
    let second = ProjectCacheIdentity::for_project(
        directory.path(),
        ProjectCacheTarget::ProjectRoot,
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;
    assert_ne!(first, second);
    Ok(())
}

#[test]
fn scene_owned_identity_excludes_child_payload_and_presentation_names() -> Result<()> {
    use usd_project::{SceneManifestEntry, SceneMember, SceneMemberId, SceneMemberTarget, ScenePlacementTransform, StorageKey};

    let directory = tempdir()?;
    let parent_scene = SceneId::new_v4();
    let child_scene = SceneId::new_v4();
    let member_id = SceneMemberId::new_v4();
    let mut manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Owned Identity",
        ProjectRoot::Scene(parent_scene),
        vec![
            SceneManifestEntry { id: parent_scene, storage_key: StorageKey::new("Parent")?, display_name: "Parent".to_owned() },
            SceneManifestEntry { id: child_scene, storage_key: StorageKey::new("Child")?, display_name: "Child".to_owned() },
        ],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    let child_path = crate::project::scene::authoring::author_scene_atomic(directory.path(), child_scene)?;
    let parent_member = SceneMember {
        id: member_id,
        target: SceneMemberTarget::Scene(child_scene),
        name: Some("Child".to_owned()),
        transform: ScenePlacementTransform::IDENTITY,
    };
    let parent_path = crate::project::scene::authoring::author_scene_atomic_with_members(
        directory.path(), parent_scene, std::slice::from_ref(&parent_member),
    )?;
    let identity = |scene_id: SceneId| ProjectCacheIdentity::for_project(
        directory.path(),
        ProjectCacheTarget::Scene { id: scene_id.to_string() },
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    );
    let parent_before = identity(parent_scene)?;
    let child_before = identity(child_scene)?;

    let stage = openusd::usd::Stage::open(child_path.to_string_lossy().as_ref())?;
    stage.define_prim("/SceneRoot/Changed")?.set_type_name("Xform")?;
    let temporary = child_path.with_extension("changed.usda");
    stage.root_layer().export(temporary.to_string_lossy().as_ref())?;
    fs::rename(temporary, &child_path)?;
    let child_after_content = identity(child_scene)?;
    assert_ne!(child_before, child_after_content);
    assert_eq!(parent_before, identity(parent_scene)?);

    crate::project::scene::authoring::update_display_name_atomic(&child_path, "/SceneRoot", "Renamed")?;
    crate::project::scene::authoring::update_member_display_name_atomic(
        &parent_path, parent_scene, member_id, "Renamed",
    )?;
    manifest.scenes.iter_mut().find(|scene| scene.id == child_scene).unwrap().display_name = "Renamed".to_owned();
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    assert_eq!(child_after_content, identity(child_scene)?);
    assert_eq!(parent_before, identity(parent_scene)?);

    let placed_member = SceneMember {
        transform: ScenePlacementTransform::from_translation([1.0, 0.0, 0.0]),
        ..parent_member
    };
    crate::project::scene::authoring::author_scene_atomic_with_members(
        directory.path(), parent_scene, &[placed_member],
    )?;
    assert_ne!(parent_before, identity(parent_scene)?);
    assert_eq!(child_after_content, identity(child_scene)?);
    Ok(())
}


#[test]
fn scene_owned_identity_does_not_import_child_model_payload() -> Result<()> {
    use usd_project::{ModelId, ModelManifestEntry, ModelSourceKind, SceneManifestEntry, SceneMember, SceneMemberId, SceneMemberTarget, ScenePlacementTransform, StorageKey};

    let directory = tempdir()?;
    let scene_id = SceneId::new_v4();
    let model_id = ModelId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Model Child Identity",
        ProjectRoot::Scene(scene_id),
        vec![SceneManifestEntry { id: scene_id, storage_key: StorageKey::new("Parent")?, display_name: "Parent".to_owned() }],
        vec![ModelManifestEntry { id: model_id, source_kind: ModelSourceKind::Usd, storage_key: StorageKey::new("ChildModel")?, display_name: "Child Model".to_owned() }],
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    let wrapper = crate::project::model_wrapper::model_wrapper_path(directory.path(), model_id);
    fs::create_dir_all(wrapper.parent().unwrap())?;
    fs::write(&wrapper, "#usda 1.0\n(defaultPrim = \"ModelRoot\")\ndef Xform \"ModelRoot\" {}\n")?;
    crate::project::scene::authoring::author_scene_atomic_with_members(
        directory.path(),
        scene_id,
        &[SceneMember {
            id: SceneMemberId::new_v4(),
            target: SceneMemberTarget::Model(model_id),
            name: Some("Child Model".to_owned()),
            transform: ScenePlacementTransform::IDENTITY,
        }],
    )?;
    let identity = |target| ProjectCacheIdentity::for_project(
        directory.path(), target, RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    );
    let parent_before = identity(ProjectCacheTarget::Scene { id: scene_id.to_string() })?;
    let model_before = identity(ProjectCacheTarget::Model { id: model_id.to_string() })?;
    let stage = openusd::usd::Stage::open(wrapper.to_string_lossy().as_ref())?;
    stage.define_prim("/ModelRoot/Changed")?.set_type_name("Xform")?;
    let temporary = wrapper.with_extension("changed.usda");
    stage.root_layer().export(temporary.to_string_lossy().as_ref())?;
    fs::rename(temporary, &wrapper)?;
    let model_after_content = identity(ProjectCacheTarget::Model { id: model_id.to_string() })?;
    assert_eq!(parent_before, identity(ProjectCacheTarget::Scene { id: scene_id.to_string() })?);
    assert_ne!(model_before, model_after_content);

    crate::project::scene::authoring::update_display_name_atomic(&wrapper, "/ModelRoot", "Renamed Model")?;
    assert_eq!(parent_before, identity(ProjectCacheTarget::Scene { id: scene_id.to_string() })?);
    assert_eq!(model_after_content, identity(ProjectCacheTarget::Model { id: model_id.to_string() })?);
    Ok(())
}
