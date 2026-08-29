use std::fs;

use anyhow::Result;
use openusd::{sdf::Value, usd::Stage};
use tempfile::tempdir;

use super::*;

#[test]
fn authored_scene_reopens_with_default_prim_and_registry_identity() -> Result<()> {
    let project_directory = tempdir()?;
    let scene_id = SceneId::new_v4();
    let path = author_scene_atomic(project_directory.path(), scene_id)?;

    assert_eq!(path, scene_path(project_directory.path(), scene_id));
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::open(&path_string)?;
    assert_eq!(
        stage.default_prim().as_ref().map(|token| token.as_str()),
        Some(SCENE_ROOT_PRIM)
    );
    assert_eq!(
        stage.stage_metadata("upAxis")?,
        Some(Value::Token("Y".into()))
    );
    assert_eq!(
        stage.stage_metadata("metersPerUnit")?,
        Some(Value::Double(1.0))
    );

    let root = stage.prim(format!("/{SCENE_ROOT_PRIM}").as_str());
    assert!(root.is_defined()?);
    let Some(Value::Dictionary(custom_data)) = root.custom_data()? else {
        panic!("SceneRoot customData should be authored");
    };
    assert_eq!(
        custom_data.get(SCENE_ID_METADATA),
        Some(&Value::String(scene_id.to_string()))
    );
    assert_eq!(
        custom_data.get(SCHEMA_VERSION_METADATA),
        Some(&Value::Int(SCENE_SCHEMA_VERSION))
    );
    validate_scene_file(&path, scene_id, &[])?;

    let scene_directory = path.parent().expect("scene directory");
    assert!(fs::read_dir(scene_directory)?.all(|entry| {
        entry
            .expect("read Project Scene directory entry")
            .file_name()
            .to_string_lossy()
            .ends_with(".usda")
    }));
    Ok(())
}

#[test]
fn repeated_targets_keep_distinct_member_ids_after_reopen() -> Result<()> {
    let project_directory = tempdir()?;
    let scene_id = SceneId::new_v4();
    let shared_scene_id = SceneId::new_v4();
    let shared_model_id = ModelId::new_v4();
    let members = vec![
        SceneMember {
            id: SceneMemberId::new_v4(),
            target: SceneMemberTarget::Scene(shared_scene_id),
            name: Some("Scene placement A".to_owned()),
            transform: Default::default(),
        },
        SceneMember {
            id: SceneMemberId::new_v4(),
            target: SceneMemberTarget::Scene(shared_scene_id),
            name: Some("Scene placement B".to_owned()),
            transform: Default::default(),
        },
        SceneMember {
            id: SceneMemberId::new_v4(),
            target: SceneMemberTarget::Model(shared_model_id),
            name: Some("Model placement A".to_owned()),
            transform: Default::default(),
        },
        SceneMember {
            id: SceneMemberId::new_v4(),
            target: SceneMemberTarget::Model(shared_model_id),
            name: Some("Model placement B".to_owned()),
            transform: Default::default(),
        },
    ];

    let path = author_scene_atomic_with_members(project_directory.path(), scene_id, &members)?;
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::open(&path_string)?;
    let reopened = members
        .iter()
        .map(|member| read_scene_member(&stage, member.id))
        .collect::<Result<Vec<_>>>()?;

    assert_eq!(reopened, members);
    assert_ne!(members[0].id, members[1].id);
    assert_ne!(members[2].id, members[3].id);
    assert_eq!(members[0].target, members[1].target);
    assert_eq!(members[2].target, members[3].target);
    Ok(())
}

#[test]
fn self_scene_placement_is_rejected_before_publication() {
    let project_directory = tempdir().unwrap();
    let scene_id = SceneId::new_v4();
    let member = SceneMember {
        id: SceneMemberId::new_v4(),
        target: SceneMemberTarget::Scene(scene_id),
        name: None,
        transform: Default::default(),
    };

    assert!(
        author_scene_atomic_with_members(project_directory.path(), scene_id, &[member]).is_err()
    );
    assert!(!scene_path(project_directory.path(), scene_id).exists());
}
