use super::*;
use crate::project::cache::{ProjectCacheIdentity, ProjectCacheTarget};
use crate::project::cache_hydration::ActiveProjectCacheContext;
use crate::viewport::session::SceneCacheOwnershipContext;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use usd_model::HashDigest;
use usd_project::SceneId;

#[test]
fn default_budget_matches_bevy_upload_throttle() {
    let budgets = ResidencyBudgets::default();
    assert_eq!(
        budgets.upload_bytes_per_frame,
        DEFAULT_UPLOAD_BYTES_PER_FRAME
    );
}

#[test]
fn lifecycle_retirement_rejects_stale_residency_completion() {
    let scene = SceneId::new_v4();
    let key = ScenePayloadKey {
        scene_id: scene,
        blob_hash: HashDigest::new([9; HashDigest::BYTE_LEN]),
    };
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, 41, Vec::new());
    assert!(authority.request_reason(key, ResidencyReason::CameraNear, 41, 8, 8));
    let stale_job = authority.begin_next_load().expect("queued camera payload");

    let mut world = World::new();
    world.insert_resource(authority);
    world.insert_resource(Assets::<Mesh>::default());
    world.init_resource::<ScenePayloadCatalog>();
    retire_scene_cache_resources(&mut world);

    assert!(world.get_resource::<ScenePayloadCatalog>().is_some());
    let mut authority = world.resource_mut::<ResidencyAuthority>();
    assert_eq!(authority.state(&key), None);
    assert_eq!(authority.accounted_bytes(), (0, 0));
    assert!(!authority.complete_cached_cpu(
        stale_job,
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default()
        ),
    ));
}

#[test]
fn scene_v3_owner_root_precedes_legacy_project_cache_root() {
    let scene = SceneId::new_v4();
    let owner_root = std::path::PathBuf::from("/tmp/scene-v3-owner");
    let legacy_root = std::path::PathBuf::from("/tmp/legacy-project-cache");
    let owner = SceneCacheOwnershipContext {
        project_root: owner_root.clone(),
        scene_id: scene,
        config_hash: HashDigest::new([1; HashDigest::BYTE_LEN]),
    };
    let legacy = ActiveProjectCacheContext::from_identity(
        legacy_root.clone(),
        ProjectCacheIdentity {
            target: ProjectCacheTarget::ProjectRoot,
            target_content_hash: HashDigest::new([2; HashDigest::BYTE_LEN]),
            profile: viewport_protocol::RuntimeProfile::NativeMedium,
            config_hash: HashDigest::new([3; HashDigest::BYTE_LEN]),
        },
    );

    assert_eq!(
        active_cache_project_root(Some(&owner), Some(&legacy)),
        Some(owner_root)
    );
    assert_eq!(
        active_cache_project_root(None, Some(&legacy)),
        Some(legacy_root)
    );
}
