use std::fs;

use tempfile::tempdir;
use usd_project::{ProjectId, ProjectManifestV1, ProjectRoot};

use super::*;
use crate::project::catalog::manifest_store::ManifestStore;

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
    let descriptor = queue
        .wait_for(directory.path(), &target)?
        .expect("empty Project warm completes");
    assert_eq!(descriptor.state, ProjectCacheState::Empty);
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
fn source_stamped_warm_keys_change_with_working_content() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    fs::write(directory.path().join("stage.usda"), b"first")?;
    let first = ProjectCacheIdentity::for_project(
        directory.path(),
        ProjectCacheTarget::ProjectRoot,
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    fs::write(directory.path().join("stage.usda"), b"second")?;
    let second = ProjectCacheIdentity::for_project(
        directory.path(),
        ProjectCacheTarget::ProjectRoot,
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    assert_ne!(identity_key(&first), identity_key(&second));
    Ok(())
}
