use std::fs;

use anyhow::Result;
use openusd::usd::Stage;

use super::*;

#[test]
fn checkpoint_round_trips_working_stage_and_metadata() -> Result<()> {
    let project_root = tempfile::tempdir()?;
    let stage = Stage::builder().in_memory("recovery-round-trip.usda")?;
    stage.define_prim("/World")?.set_type_name("Xform")?;
    let live = LiveStage::new(stage);
    usd_bevy::authoring::define_prim(&live.stage, "/World/Recovered", "Cube")?;
    let revision = live
        .drain_change_batch()
        .expect("authoring should create a live revision")
        .revision;
    let store = RecoveryStore::new(project_root.path(), live.session_id())?;

    let metadata = store.write_checkpoint(
        &live,
        Path::new("assets/scene.usda"),
        Some("head-before-edit"),
    )?;
    assert_eq!(metadata.live_revision, revision.0);
    assert_eq!(metadata.base_revision.as_deref(), Some("head-before-edit"));
    assert!(store.stage_path().is_file());
    assert!(store.metadata_path().is_file());

    let inspected = store.inspect()?.expect("checkpoint should exist");
    assert_eq!(inspected, metadata);
    let restored = store.restore()?.expect("checkpoint should restore");
    assert_eq!(restored.metadata, metadata);
    assert!(usd_bevy::authoring::prim_exists(
        &restored.stage,
        "/World/Recovered"
    ));

    assert!(store.clear()?);
    assert!(store.restore()?.is_none());
    Ok(())
}

#[test]
fn corrupt_or_partial_checkpoint_is_rejected() -> Result<()> {
    let project_root = tempfile::tempdir()?;
    let stage = Stage::builder().in_memory("recovery-corrupt.usda")?;
    stage.define_prim("/World")?.set_type_name("Xform")?;
    let live = LiveStage::new(stage);
    let store = RecoveryStore::new(project_root.path(), live.session_id())?;
    store.write_checkpoint(&live, Path::new("scene.usda"), None)?;

    fs::write(store.stage_path(), b"#usda 1.0\ndef Xform \"Wrong\" {}\n")?;
    let error = match store.restore() {
        Err(error) => error,
        Ok(_) => panic!("digest mismatch should fail"),
    };
    assert!(error.to_string().contains("digest mismatch"));

    store.clear()?;
    fs::create_dir_all(store.directory())?;
    fs::write(store.stage_path(), b"partial")?;
    let error = store.inspect().expect_err("partial checkpoint should fail");
    assert!(error.to_string().contains("incomplete"));
    Ok(())
}
