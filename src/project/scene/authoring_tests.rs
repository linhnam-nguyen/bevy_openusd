use std::fs;

use anyhow::Result;
use openusd::{gf::Vec3f, sdf::Value, usd::Stage};
use tempfile::tempdir;
use usd_project::ScenePlacementTransform;

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
fn schema_v2_authors_direct_members_and_relative_project_references() -> Result<()> {
    let project_directory = tempdir()?;
    let scene_id = SceneId::new_v4();
    let scene_path = project_directory.path().join("scenes/Parent.usda");
    let child_scene_path = project_directory.path().join("Child.usda");
    let model_path = project_directory.path().join("models/Chair/model.usda");
    let members = vec![
        SceneMember {
            id: SceneMemberId::new_v4(),
            target: SceneMemberTarget::Scene(SceneId::new_v4()),
            name: Some("Child scene".to_owned()),
            transform: Default::default(),
        },
        SceneMember {
            id: SceneMemberId::new_v4(),
            target: SceneMemberTarget::Model(ModelId::new_v4()),
            name: Some("Chair".to_owned()),
            transform: Default::default(),
        },
    ];

    let stage = new_scene_stage(scene_id)?;
    author_scene_member_at_path(
        &stage,
        project_directory.path(),
        &scene_path,
        &members[0],
        Some(&child_scene_path),
    )?;
    author_scene_member_at_path(
        &stage,
        project_directory.path(),
        &scene_path,
        &members[1],
        Some(&model_path),
    )?;
    fs::create_dir_all(scene_path.parent().expect("Scene directory"))?;
    stage
        .root_layer()
        .export(scene_path.to_string_lossy().as_ref())?;

    let reopened = Stage::open(scene_path.to_string_lossy().as_ref())?;
    assert!(!reopened.prim("/SceneRoot/Members").is_defined()?);
    assert!(
        reopened
            .prim(scene_member_path(members[0].id).as_str())
            .is_defined()?
    );
    assert!(
        reopened
            .prim(scene_member_path(members[1].id).as_str())
            .is_defined()?
    );
    let mut expected_members = members.clone();
    expected_members.sort_by_key(|member| member.id);
    assert_eq!(read_scene_members(&scene_path, scene_id)?, expected_members);

    for (member, expected_asset) in [
        (&members[0], "../Child.usda"),
        (&members[1], "../models/Chair/model.usda"),
    ] {
        let spec_path = sdf::path(scene_member_path(member.id).as_str())?;
        let root_layer = reopened.root_layer();
        let spec = root_layer.prim(&spec_path).expect("member spec");
        let Some(Value::ReferenceListOp(references)) = spec.field(REFERENCES_FIELD)? else {
            panic!("direct member reference list should be authored");
        };
        assert_eq!(
            references
                .iter()
                .next()
                .expect("member reference")
                .asset_path,
            expected_asset
        );
    }
    validate_scene_file(&scene_path, scene_id, &members)?;
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
fn non_identity_scene_placement_round_trips_through_usd() -> Result<()> {
    let project_directory = tempdir()?;
    let scene_id = SceneId::new_v4();
    let member = SceneMember {
        id: SceneMemberId::new_v4(),
        target: SceneMemberTarget::Model(ModelId::new_v4()),
        name: Some("Translated model placement".to_owned()),
        transform: ScenePlacementTransform::from_translation([10.0, 20.0, 30.0]),
    };

    let path =
        author_scene_atomic_with_members(project_directory.path(), scene_id, &[member.clone()])?;
    let reopened = read_scene_members(&path, scene_id)?;

    assert_eq!(reopened, vec![member.clone()]);
    let stage = Stage::open(path.to_string_lossy().as_ref())?;
    let value = stage
        .prim(scene_member_path(member.id).as_str())
        .attribute("xformOp:transform")
        .get::<Value>()?
        .expect("authored placement matrix");
    let Value::Matrix4d(matrix) = value else {
        panic!("placement transform should be matrix4d");
    };
    let origin = matrix.transform_point(Vec3f {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
    assert_eq!([origin.x, origin.y, origin.z], [10.0, 20.0, 30.0]);
    Ok(())
}

#[test]
fn asymmetric_trs_placement_preserves_row_vector_rotation_and_scale() {
    let transform = ScenePlacementTransform::from_trs(
        [7.0, -3.0, 11.0],
        [
            std::f64::consts::FRAC_1_SQRT_2,
            0.0,
            0.0,
            std::f64::consts::FRAC_1_SQRT_2,
        ],
        [2.0, 3.0, 5.0],
    );
    let matrix = openusd::gf::Matrix4d(transform.0);
    let transformed = matrix.transform_point(Vec3f {
        x: 1.0,
        y: 2.0,
        z: 0.0,
    });

    assert!((transformed.x - 1.0).abs() < 1e-5);
    assert!((transformed.y + 1.0).abs() < 1e-5);
    assert!((transformed.z - 11.0).abs() < 1e-5);
    assert_eq!(transform.0[12..15], [7.0, -3.0, 11.0]);
}

#[test]
fn legacy_scene_placement_without_transform_defaults_to_identity() -> Result<()> {
    let project_directory = tempdir()?;
    let scene_id = SceneId::new_v4();
    let member = SceneMember {
        id: SceneMemberId::new_v4(),
        target: SceneMemberTarget::Model(ModelId::new_v4()),
        name: None,
        transform: Default::default(),
    };
    let path = scene_path(project_directory.path(), scene_id);
    fs::create_dir_all(path.parent().expect("Scene directory"))?;
    let stage = new_scene_stage(scene_id)?;
    let root = stage.prim(format!("/{SCENE_ROOT_PRIM}").as_str());
    let mut root_data = match root.custom_data()? {
        Some(Value::Dictionary(data)) => data,
        _ => panic!("SceneRoot customData should be authored"),
    };
    root_data.insert(
        SCHEMA_VERSION_METADATA.to_owned(),
        Value::Int(LEGACY_SCENE_SCHEMA_VERSION),
    );
    root.set_metadata("customData", Value::Dictionary(root_data))?;
    stage
        .define_prim(format!("/{SCENE_ROOT_PRIM}/{SCENE_MEMBERS_PRIM}").as_str())?
        .set_type_name("Xform")?;
    stage
        .define_prim(legacy_scene_member_path(member.id).as_str())?
        .set_type_name("Xform")?
        .set_metadata("customData", Value::Dictionary(member_custom_data(&member)))?;
    stage.root_layer().export(path.to_string_lossy().as_ref())?;

    assert_eq!(read_scene_members(&path, scene_id)?, vec![member]);
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
