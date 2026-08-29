use std::fs;

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
fn affected_scene_targets_include_composed_ancestors_and_root() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let project_id = usd_project::ProjectId::new_v4();
    let root_scene = usd_project::SceneId::new_v4();
    let child_scene = usd_project::SceneId::new_v4();
    let manifest = usd_project::ProjectManifestV1::new(
        project_id,
        "Warm Project",
        usd_project::ProjectRoot::Scene(root_scene),
        vec![
            usd_project::SceneManifestEntry {
                id: root_scene,
                storage_key: usd_project::StorageKey::new("root").unwrap(),
            },
            usd_project::SceneManifestEntry {
                id: child_scene,
                storage_key: usd_project::StorageKey::new("child").unwrap(),
            },
        ],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    crate::project::scene::authoring::author_scene_atomic_with_members(
        directory.path(),
        root_scene,
        &[usd_project::SceneMember {
            id: usd_project::SceneMemberId::new_v4(),
            target: usd_project::SceneMemberTarget::Scene(child_scene),
            name: None,
            transform: Default::default(),
        }],
    )?;
    crate::project::scene::authoring::author_scene_atomic_with_members(
        directory.path(),
        child_scene,
        &[],
    )?;

    let targets = affected_targets(
        directory.path(),
        &ProjectCacheTarget::Scene {
            id: child_scene.to_string(),
        },
    )?;
    let keys = targets
        .into_iter()
        .map(|target| target.key())
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            format!("scene:{child_scene}"),
            format!("scene:{root_scene}"),
            "project".to_owned(),
        ]
    );
    Ok(())
}

#[test]
fn affected_model_targets_include_composed_scene_ancestors_but_not_siblings() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let project_id = usd_project::ProjectId::new_v4();
    let root_scene = usd_project::SceneId::new_v4();
    let child_scene = usd_project::SceneId::new_v4();
    let sibling_scene = usd_project::SceneId::new_v4();
    let model_id = usd_project::ModelId::new_v4();
    let manifest = usd_project::ProjectManifestV1::new(
        project_id,
        "Warm Project",
        usd_project::ProjectRoot::Scene(root_scene),
        vec![
            usd_project::SceneManifestEntry {
                id: root_scene,
                storage_key: usd_project::StorageKey::new("root").unwrap(),
            },
            usd_project::SceneManifestEntry {
                id: child_scene,
                storage_key: usd_project::StorageKey::new("child").unwrap(),
            },
            usd_project::SceneManifestEntry {
                id: sibling_scene,
                storage_key: usd_project::StorageKey::new("sibling").unwrap(),
            },
        ],
        vec![usd_project::ModelManifestEntry {
            id: model_id,
            source_kind: usd_project::ModelSourceKind::Usd,
            storage_key: usd_project::StorageKey::new("model").unwrap(),
        }],
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    crate::project::scene::authoring::author_scene_atomic_with_members(
        directory.path(),
        root_scene,
        &[usd_project::SceneMember {
            id: usd_project::SceneMemberId::new_v4(),
            target: usd_project::SceneMemberTarget::Scene(child_scene),
            name: None,
            transform: Default::default(),
        }],
    )?;
    crate::project::scene::authoring::author_scene_atomic_with_members(
        directory.path(),
        child_scene,
        &[usd_project::SceneMember {
            id: usd_project::SceneMemberId::new_v4(),
            target: usd_project::SceneMemberTarget::Model(model_id),
            name: None,
            transform: Default::default(),
        }],
    )?;
    crate::project::scene::authoring::author_scene_atomic_with_members(
        directory.path(),
        sibling_scene,
        &[],
    )?;

    let targets = affected_targets(
        directory.path(),
        &ProjectCacheTarget::Model {
            id: model_id.to_string(),
        },
    )?;
    let keys = targets
        .into_iter()
        .map(|target| target.key())
        .collect::<Vec<_>>();
    let mut expected = vec![
        format!("model:{model_id}"),
        format!("scene:{child_scene}"),
        format!("scene:{root_scene}"),
        "project".to_owned(),
    ];
    expected[1..3].sort();
    assert_eq!(keys, expected);
    assert!(!keys.contains(&format!("scene:{sibling_scene}")));
    Ok(())
}

#[test]
fn target_content_warm_keys_change_with_target_content() -> Result<()> {
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
        SemanticConfig::default().hash(),
    )?;
    let mut changed_scene = fs::read(&scene_path)?;
    changed_scene.extend_from_slice(b"\n# target content changed\n");
    fs::write(&scene_path, changed_scene)?;
    let second = ProjectCacheIdentity::for_project(
        directory.path(),
        ProjectCacheTarget::ProjectRoot,
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    assert_ne!(identity_key(&first), identity_key(&second));
    Ok(())
}

#[test]
fn unrelated_scene_edits_keep_a_sibling_cache_identity_reusable() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Warm Project",
        ProjectRoot::Empty,
        vec![
            usd_project::SceneManifestEntry {
                id: SceneId::new_v4(),
                storage_key: usd_project::StorageKey::new("first").unwrap(),
            },
            usd_project::SceneManifestEntry {
                id: SceneId::new_v4(),
                storage_key: usd_project::StorageKey::new("second").unwrap(),
            },
        ],
        Vec::new(),
    );
    let first_scene = manifest.scenes[0].id;
    let second_scene = manifest.scenes[1].id;
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    let first_path =
        crate::project::scene::authoring::author_scene_atomic(directory.path(), first_scene)?;
    let second_path =
        crate::project::scene::authoring::author_scene_atomic(directory.path(), second_scene)?;
    let first_identity = ProjectCacheIdentity::for_project(
        directory.path(),
        ProjectCacheTarget::Scene {
            id: first_scene.to_string(),
        },
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    let store = ProjectCacheStore::new(directory.path());
    store.publish(&ProjectCacheDescriptor::new(
        first_identity.clone(),
        ProjectCacheState::Partial,
        None,
    )?)?;

    fs::write(second_path, b"unrelated sibling edit")?;
    let unchanged_identity = ProjectCacheIdentity::for_project(
        directory.path(),
        ProjectCacheTarget::Scene {
            id: first_scene.to_string(),
        },
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    assert_eq!(first_identity, unchanged_identity);
    assert!(store.load(&unchanged_identity)?.is_some());
    assert!(first_path.is_file());
    Ok(())
}
