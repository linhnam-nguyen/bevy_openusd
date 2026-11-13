use openusd::sdf::Value;
use openusd::usd::Stage;

use super::*;

fn stage_with(root: &str) -> Stage {
    let stage = Stage::builder().in_memory("authoring_test.usda").unwrap();
    stage
        .define_prim(root)
        .unwrap()
        .set_type_name("Xform")
        .unwrap();
    stage
}

#[test]
fn define_and_remove() {
    let stage = stage_with("/World");
    define_prim(&stage, "/World/Box", "Cube").unwrap();
    assert_eq!(
        stage
            .prim(openusd::sdf::path("/World/Box").unwrap())
            .type_name()
            .unwrap()
            .as_deref(),
        Some("Cube")
    );
    assert!(remove_prim(&stage, "/World/Box").unwrap());
    assert!(
        stage
            .prim(openusd::sdf::path("/World/Box").unwrap())
            .type_name()
            .unwrap()
            .is_none(),
        "removed prim is gone"
    );
}

#[test]
fn rename_and_reparent() {
    let stage = stage_with("/World");
    define_prim(&stage, "/World/A", "Xform").unwrap();
    define_prim(&stage, "/World/B", "Xform").unwrap();
    define_prim(&stage, "/World/A/Child", "Cube").unwrap();

    rename_prim(&stage, "/World/A", "Renamed").unwrap();
    assert!(
        stage
            .prim(openusd::sdf::path("/World/Renamed").unwrap())
            .type_name()
            .unwrap()
            .is_some(),
        "rename created /World/Renamed"
    );
    assert!(
        stage
            .prim(openusd::sdf::path("/World/A").unwrap())
            .type_name()
            .unwrap()
            .is_none(),
        "old /World/A is gone"
    );

    reparent_prim(&stage, "/World/Renamed/Child", "/World/B").unwrap();
    assert!(
        stage
            .prim(openusd::sdf::path("/World/B/Child").unwrap())
            .type_name()
            .unwrap()
            .is_some(),
        "child reparented under /World/B"
    );
}

#[test]
fn set_attribute_roundtrips() {
    let stage = stage_with("/World");
    set_attribute(&stage, "/World", "radius", "double", Value::Double(2.5)).unwrap();
    let got = stage
        .prim(openusd::sdf::path("/World").unwrap())
        .attribute("radius")
        .get::<Value>()
        .unwrap();
    assert!(matches!(got, Some(Value::Double(d)) if (d - 2.5).abs() < 1e-9));
}

#[test]
fn edit_history_undo_redo() {
    let stage = stage_with("/World");
    let mut hist = EditHistory::default();

    // Define → undo removes → redo re-creates.
    hist.define(&stage, "/World/Box", "Cube").unwrap();
    assert!(prim_exists(&stage, "/World/Box"));
    assert!(hist.undo(&stage).unwrap());
    assert!(!prim_exists(&stage, "/World/Box"), "undo removed the prim");
    assert!(hist.redo(&stage).unwrap());
    assert!(prim_exists(&stage, "/World/Box"), "redo re-created it");

    // SetAttr captures the prior value for undo.
    hist.set_attr(&stage, "/World/Box", "size", "double", Value::Double(1.0))
        .unwrap();
    hist.set_attr(&stage, "/World/Box", "size", "double", Value::Double(9.0))
        .unwrap();
    let read = |s: &Stage| {
        s.prim(openusd::sdf::path("/World/Box").unwrap())
            .attribute("size")
            .get::<Value>()
            .unwrap()
    };
    assert!(matches!(read(&stage), Some(Value::Double(d)) if (d - 9.0).abs() < 1e-9));
    hist.undo(&stage).unwrap();
    assert!(
        matches!(read(&stage), Some(Value::Double(d)) if (d - 1.0).abs() < 1e-9),
        "undo → prior value"
    );

    // Rename → undo restores the original name.
    hist.rename(&stage, "/World/Box", "Crate").unwrap();
    assert!(prim_exists(&stage, "/World/Crate"));
    hist.undo(&stage).unwrap();
    assert!(prim_exists(&stage, "/World/Box"), "undo restored the name");
    assert!(!prim_exists(&stage, "/World/Crate"));
}

#[test]
fn atomic_attribute_edit_undoes_and_redoes_as_one_group() {
    let stage = stage_with("/World");
    define_prim(&stage, "/World/A", "Xform").unwrap();
    define_prim(&stage, "/World/B", "Xform").unwrap();
    let mut hist = EditHistory::default();

    hist.set_attrs_atomic(
        &stage,
        &[
            AttributeEdit {
                prim: "/World/A".into(),
                name: "Width".into(),
                type_name: "double".into(),
                value: Value::Double(2.0),
            },
            AttributeEdit {
                prim: "/World/B".into(),
                name: "Width".into(),
                type_name: "double".into(),
                value: Value::Double(3.0),
            },
        ],
    )
    .unwrap();
    assert!(hist.can_undo());

    let read = |path: &str| {
        stage
            .prim(openusd::sdf::path(path).unwrap())
            .attribute("Width")
            .get::<Value>()
            .unwrap()
    };
    assert!(matches!(read("/World/A"), Some(Value::Double(value)) if value == 2.0));
    assert!(matches!(read("/World/B"), Some(Value::Double(value)) if value == 3.0));

    assert!(hist.undo(&stage).unwrap());
    assert!(read("/World/A").is_none());
    assert!(read("/World/B").is_none());
    assert!(hist.redo(&stage).unwrap());
    assert!(matches!(read("/World/A"), Some(Value::Double(value)) if value == 2.0));
    assert!(matches!(read("/World/B"), Some(Value::Double(value)) if value == 3.0));
}

#[test]
fn persistence_export_and_reopen() {
    let stage = stage_with("/World");
    define_prim(&stage, "/World/Saved", "Sphere").unwrap();
    set_attribute(
        &stage,
        "/World/Saved",
        "radius",
        "double",
        Value::Double(3.0),
    )
    .unwrap();

    // String export mentions the authored prim.
    let usda = export_stage_string(&stage).unwrap();
    assert!(
        usda.contains("Saved"),
        "export should contain the prim, got:\n{usda}"
    );

    // File export round-trips through a fresh open.
    let path = std::env::temp_dir().join("usd_bevy_persist_test.usda");
    let path_str = path.to_str().unwrap();
    save_stage_as(&stage, path_str).unwrap();
    let reopened = Stage::open(path_str).unwrap();
    assert!(
        reopened
            .prim(openusd::sdf::path("/World/Saved").unwrap())
            .type_name()
            .unwrap()
            .is_some(),
        "reopened stage has the saved prim"
    );
    let _ = std::fs::remove_file(&path);
}
