use std::collections::HashMap;

use openusd::{sdf::Value, usd::Stage};
use usd_project::ModelId;
use viewport_protocol::{RuntimeMutation, RuntimeMutationBatch, ViewportCommand};

use super::validate_command;

fn model_stage() -> (Stage, ModelId) {
    let stage = Stage::builder()
        .in_memory("model-boundary.usda")
        .expect("model boundary stage");
    let model_id = ModelId::new_v4();
    stage
        .define_prim("/ModelRoot")
        .expect("model root")
        .set_metadata(
            "customData",
            Value::Dictionary(HashMap::from([(
                "usdhub:modelId".to_owned(),
                Value::String(model_id.to_string()),
            )])),
        )
        .expect("model identity");
    stage
        .define_prim("/ModelRoot/Source/Body")
        .expect("model source");
    stage.define_prim("/SceneRoot").expect("Scene root");
    stage
        .define_prim("/SceneRoot/ModelMember")
        .expect("model member")
        .set_metadata(
            "customData",
            Value::Dictionary(HashMap::from([
                (
                    "usdhub:targetKind".to_owned(),
                    Value::String("model".to_owned()),
                ),
                (
                    "usdhub:targetId".to_owned(),
                    Value::String(model_id.to_string()),
                ),
            ])),
        )
        .expect("model member identity");
    (stage, model_id)
}

#[test]
fn model_source_mutations_are_rejected_by_identity() {
    let (stage, _) = model_stage();
    let command = ViewportCommand::SetAttribute {
        prim_path: "/ModelRoot/Source/Body".to_owned(),
        name: "locked".to_owned(),
        type_name: "bool".to_owned(),
        value: serde_json::json!(true),
    };
    let error = validate_command(&stage, &command).expect_err("Model source must be immutable");
    assert!(error.contains("Model source is immutable"));
}

#[test]
fn model_member_root_remains_editable_but_its_source_does_not() {
    let (stage, _) = model_stage();
    let member = stage.prim("/SceneRoot/ModelMember");
    assert!(member.is_defined().expect("model member is defined"));
    assert!(
        member
            .custom_data()
            .expect("model member custom data")
            .is_some()
    );
    let transform = ViewportCommand::SetTransform {
        prim_path: "/SceneRoot/ModelMember".to_owned(),
        translation: [1.0, 2.0, 3.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0, 1.0, 1.0],
    };
    assert!(validate_command(&stage, &transform).is_ok());
    let define = ViewportCommand::DefinePrim {
        path: "/SceneRoot/ModelMember/Override".to_owned(),
        type_name: "Xform".to_owned(),
    };
    assert!(validate_command(&stage, &define).is_err());
}

#[test]
fn runtime_batches_are_validated_before_model_source_authoring() {
    let (stage, _) = model_stage();
    let command = ViewportCommand::ApplyRuntimeMutationBatch {
        batch: RuntimeMutationBatch {
            source_id: "test-runtime".to_owned(),
            sequence: 1,
            base_revision: 0,
            operations: vec![RuntimeMutation::RemovePrim {
                path: "/ModelRoot/Source/Body".to_owned(),
            }],
        },
    };
    assert!(validate_command(&stage, &command).is_err());
}
