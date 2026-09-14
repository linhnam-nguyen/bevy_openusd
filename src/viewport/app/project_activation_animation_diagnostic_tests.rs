use std::{fs, path::Path};

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, Mesh3d, PrimitiveTopology},
    prelude::Entity,
};
use openusd::usd::{PrimPredicate, Stage};
use project_protocol::{ProjectActivationCommand, ProjectStageTarget};
use tempfile::tempdir;
use usd_model::{Bounds3, HashDigest, TransformSignature};
use usd_project::{
    ProjectId, ProjectManifestV1, ProjectRoot, SceneId, SceneManifestEntry, StorageKey,
};
use viewport_protocol::{
    RuntimeBlobReference, RuntimeManifest, RuntimePayloadKind, RuntimeProfile,
};

use super::ProductionActivationWorld;
use crate::project::blob_store::{
    BlobStore, FilesystemBlobStore, OBJECTS_DIRECTORY, prepare_mesh_payload,
};
use crate::project::cache::{
    ProjectCacheIdentity, ProjectCacheState, ProjectCacheStore, ProjectCacheTarget,
};
use crate::project::cache_hydration::default_project_cache_config_hash;
use crate::project::catalog::manifest_store::ManifestStore;
use crate::project::runtime_delivery::{
    RUNTIME_HIERARCHY_VERSION, RUNTIME_MESH_VERSION, RuntimeHierarchyBlob, RuntimeHierarchyEntity,
    RuntimeHierarchyGeometry,
};
use crate::project::scene::{adoption_authoring, authoring};
use crate::project::service::{ProjectStageActivationTarget, ProjectStagePresentationContext};

#[test]
fn cached_hummingbird_diagnostic_reports_skinned_paths() {
    let fixture = CachedHummingbirdFixture::new();
    let paths = skinned_paths(&fixture.scene_path);
    eprintln!("[b0-m0+3] skinned_paths={paths:?}");
    assert!(!paths.is_empty(), "Hummingbird exposes a skinned mesh path");
}

#[test]
fn cached_hummingbird_native_binding_replaces_seeded_meshes() {
    let fixture = CachedHummingbirdFixture::new();
    let identity = ProjectCacheIdentity::for_project(
        &fixture.project_root,
        ProjectCacheTarget::ProjectRoot,
        viewport_protocol::RuntimeProfile::NativeMedium,
        default_project_cache_config_hash(),
    )
    .expect("compute Project cache identity");
    let skinned_path = skinned_paths(&fixture.scene_path)
        .into_iter()
        .next()
        .expect("Hummingbird skinned mesh");
    publish_single_mesh_seed_cache(&fixture, &identity, &skinned_path);

    let command = ProjectActivationCommand::new(
        "cached-hummingbird-animation-diagnostic",
        1,
        fixture.project_id,
        ProjectStageTarget::Scene(fixture.scene_id),
    );
    let target = ProjectStageActivationTarget {
        project_id: fixture.project_id,
        target: command.target.clone(),
        project_root: fixture.project_root.clone(),
        path: fs::canonicalize(&fixture.scene_path).expect("canonical scene wrapper"),
        archive_paths: vec![fs::canonicalize(&fixture.package_path).expect("canonical package")],
        cache_identity: Some(identity),
        scene_cache: None,
        presentation: ProjectStagePresentationContext::default(),
    };

    let mut production = ProductionActivationWorld::new();
    assert!(production.admit("cached-hummingbird-session", &command));
    let reply = production
        .apply("cached-hummingbird-session", &command, Ok(Some(target)))
        .expect("cached activation replies");
    assert!(matches!(
        reply.result,
        project_protocol::ProjectActivationResult::Activated { .. }
    ));
    let seeded_mesh_count = production
        .world()
        .resource::<usd_bevy::ProjectionSeed>()
        .pending_meshes();
    assert!(
        seeded_mesh_count > 0,
        "Project activation hydrated mesh seeds"
    );

    let mut update_ticks = 0;
    for _ in 0..10_000 {
        update_ticks += 1;
        production.update();
        if production
            .world()
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness()
            == usd_bevy::ProjectionReadiness::Ready
        {
            break;
        }
    }
    let world = production.world_mut();
    assert!(update_ticks > 1, "cached projection remained incremental");
    assert_eq!(
        world
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness(),
        usd_bevy::ProjectionReadiness::Ready
    );
    assert_eq!(
        world
            .resource::<usd_bevy::ProjectionSeed>()
            .pending_meshes(),
        0,
        "all cached seeds were consumed by projection"
    );
    assert!(
        world
            .get_resource::<crate::viewport::session::SceneCachePresentation>()
            .is_none()
    );
    assert!(
        world
            .get_resource::<crate::viewport::session::PendingCanonicalVisualHandoff>()
            .is_none()
    );
    assert_eq!(
        world
            .resource::<crate::viewport::residency::SceneResidencyProjection>()
            .active_entity_count_for_test(),
        0
    );
    assert_eq!(
        world
            .query::<&crate::viewport::residency::SceneResidencyOccurrence>()
            .iter(world)
            .count(),
        0
    );
    assert!(world.resource::<usd_bevy::PrimEntities>().len() > 0);
    assert!(!world.resource::<usd_bevy::AnimatedPrims>().0.is_empty());

    let mut mesh_entities = world.query::<(Entity, &Mesh3d, &usd_bevy::prim_ref::UsdPrimRef)>();
    let mesh_count = mesh_entities.iter(world).count();
    let mut skinned_count = 0;
    let mut seeded_entity = None;
    for (entity, _, prim) in mesh_entities.iter(world) {
        if prim.path == skinned_path {
            seeded_entity = Some(entity);
        }
        if world
            .get::<bevy::mesh::skinning::SkinnedMesh>(entity)
            .is_some()
        {
            skinned_count += 1;
        }
    }
    let joint_count = world
        .query::<&usd_bevy::route::skel::UsdJoint>()
        .iter(world)
        .count();
    eprintln!(
        "[b0-m0+3] cached_hummingbird seed_meshes={seeded_mesh_count} mesh_entities={mesh_count} skinned_meshes={skinned_count} joints={joint_count}"
    );
    assert!(mesh_count > 0, "cached Project produced renderable meshes");
    assert!(joint_count > 0, "cached Project produced native joints");
    assert!(skinned_count > 0, "cached Project attached native skinning");
    let seeded_entity = seeded_entity.expect("seeded prim resolves to a canonical mesh entity");
    let seeded_skin = world
        .get::<bevy::mesh::skinning::SkinnedMesh>(seeded_entity)
        .expect("seeded prim has native skinning");
    assert!(
        !seeded_skin.joints.is_empty(),
        "seeded prim has non-empty native skin joints"
    );
}

fn skinned_paths(scene_path: &Path) -> Vec<String> {
    let stage = Stage::open(&scene_path.to_string_lossy()).expect("open scene wrapper");
    let mut paths = Vec::new();
    stage
        .traverse(PrimPredicate::ALL, |path| {
            if usd_bevy::read::skel::is_skinned(&stage, path) {
                paths.push(path.as_str().to_owned());
            }
        })
        .expect("traverse Hummingbird scene");
    paths
}

fn publish_single_mesh_seed_cache(
    fixture: &CachedHummingbirdFixture,
    identity: &ProjectCacheIdentity,
    prim_path: &str,
) {
    let store = FilesystemBlobStore::new(fixture.project_root.join(OBJECTS_DIRECTORY))
        .expect("create cache object store");
    let mut mesh = bevy::mesh::Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        bevy::mesh::Mesh::ATTRIBUTE_POSITION,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
    let prepared = prepare_mesh_payload(&mesh).expect("encode cache mesh");
    assert_eq!(
        store.put(&prepared.bytes).expect("persist cache mesh"),
        prepared.blob_id
    );
    let hierarchy = RuntimeHierarchyBlob {
        version: RUNTIME_HIERARCHY_VERSION,
        revision: "b0-m0+3-diagnostic".to_owned(),
        entities: vec![RuntimeHierarchyEntity {
            entity_key: prim_path.to_owned(),
            prim_path: prim_path.to_owned(),
            display_name: None,
            transform: TransformSignature {
                translation_mm: [0; 3],
                rotation_quantized: [0, 0, 0, 10_000],
                scale_quantized: [10_000; 3],
                hash: HashDigest::new([0; HashDigest::BYTE_LEN]),
            },
            geometry: Some(RuntimeHierarchyGeometry {
                blob_id: prepared.blob_id.0.clone(),
                vertex_count: 3,
                index_count: 3,
                local_bounds: Bounds3 {
                    min: [0.0; 3],
                    max: [1.0; 3],
                },
            }),
            material_blob_id: None,
        }],
    };
    let hierarchy_bytes = serde_json::to_vec(&hierarchy).expect("encode cache hierarchy");
    let hierarchy_id = store
        .put(&hierarchy_bytes)
        .expect("persist cache hierarchy");
    let runtime = RuntimeManifest {
        revision: "b0-m0+3-diagnostic".to_owned(),
        profile: RuntimeProfile::NativeMedium,
        hierarchy: RuntimeBlobReference {
            blob_id: hierarchy_id.0,
            payload_kind: RuntimePayloadKind::Hierarchy,
            payload_version: RUNTIME_HIERARCHY_VERSION,
            byte_size: hierarchy_bytes.len() as u64,
        },
        meshes: vec![RuntimeBlobReference {
            blob_id: prepared.blob_id.0,
            payload_kind: RuntimePayloadKind::Mesh,
            payload_version: RUNTIME_MESH_VERSION,
            byte_size: prepared.bytes.len() as u64,
        }],
        materials: Vec::new(),
        textures: Vec::new(),
    };
    ProjectCacheStore::new(&fixture.project_root)
        .publish(
            &crate::project::cache::ProjectCacheDescriptor::new(
                identity.clone(),
                ProjectCacheState::Ready,
                Some(runtime),
            )
            .expect("create ready cache descriptor"),
        )
        .expect("publish ready cache descriptor");
}

struct CachedHummingbirdFixture {
    _directory: tempfile::TempDir,
    project_id: ProjectId,
    scene_id: SceneId,
    project_root: std::path::PathBuf,
    scene_path: std::path::PathBuf,
    package_path: std::path::PathBuf,
}

impl CachedHummingbirdFixture {
    fn new() -> Self {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/external/hummingbird.usdz");
        let directory = tempdir().expect("temporary Hummingbird Project");
        let project_root = directory.path().join("project");
        usd_git::Repository::init(&project_root).expect("initialize Hummingbird Project");
        let project_id = ProjectId::new_v4();
        let scene_id = SceneId::new_v4();
        let manifest = ProjectManifestV1::new(
            project_id,
            "Cached Hummingbird Project",
            ProjectRoot::Scene(scene_id),
            vec![SceneManifestEntry {
                id: scene_id,
                storage_key: StorageKey::new("hummingbird").expect("Hummingbird storage key"),
                display_name: "Hummingbird".to_owned(),
            }],
            Vec::new(),
        );
        ManifestStore::write_manifest_atomic(&project_root, &manifest)
            .expect("write Hummingbird manifest");
        let scene_path = authoring::scene_path(&project_root, scene_id);
        let package_dir = project_root
            .join("imports/scenes")
            .join(scene_id.to_string());
        fs::create_dir_all(&package_dir).expect("create Hummingbird import directory");
        let package_path = package_dir.join("hummingbird.usdz");
        fs::copy(source, &package_path).expect("copy Hummingbird package");
        let spatial = crate::project::spatial::inspect_source(&package_path)
            .expect("inspect Hummingbird source metadata");
        fs::create_dir_all(scene_path.parent().expect("scene directory"))
            .expect("create scene directory");
        adoption_authoring::author_scene_wrapper_to_path(
            &scene_path,
            &project_root,
            &scene_path,
            scene_id,
            &package_path,
            &package_path,
            &["/hummingbird_anim_hover_idle_long".to_owned()],
            "Hummingbird",
            &spatial,
            false,
        )
        .expect("write Hummingbird scene wrapper");
        Self {
            _directory: directory,
            project_id,
            scene_id,
            project_root,
            scene_path,
            package_path,
        }
    }
}
