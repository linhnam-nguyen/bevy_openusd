use crate::project::scene::timeline::{publish_staged_layers_atomic, restore_timeline_preimage};
use anyhow::Result;
use std::fs;

#[test]
fn later_layer_publish_failure_restores_exact_earlier_layer_bytes() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let tx = directory.path().join("transaction");
    fs::create_dir(&tx)?;
    let first = directory.path().join("first.usda");
    let second = directory.path().join("second.usda");
    let staged_first = tx.join("first.tmp.usda");
    let staged_second = tx.join("missing.tmp.usda");
    fs::write(&first, b"old first\n")?;
    fs::write(&second, b"old second\n")?;
    fs::write(&staged_first, b"new first\n")?;
    let result = publish_staged_layers_atomic(&tx, &[
        (0, staged_first, first.clone()), (1, staged_second, second.clone()),
    ]);
    assert!(result.is_err(), "missing second candidate must fail publication");
    assert_eq!(fs::read(first)?, b"old first\n");
    assert_eq!(fs::read(second)?, b"old second\n");
    Ok(())
}

#[test]
fn failed_preimage_restoration_never_consumes_the_backup() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let blocked_destination = directory.path().join("blocked");
    fs::create_dir(&blocked_destination)?;
    let backup = directory.path().join("backup.usda");
    fs::write(&backup, b"recoverable original")?;
    assert!(restore_timeline_preimage(&blocked_destination, &backup).is_err());
    assert_eq!(fs::read(&backup)?, b"recoverable original");
    assert!(blocked_destination.is_dir());
    Ok(())
}
