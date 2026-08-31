use super::*;

#[test]
fn interrupted_model_publication_preserves_the_legacy_wrapper() -> Result<()> {
    let _guard = MIGRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = legacy_fixture();
    let root = fixture.directory.path();
    let layout = ProjectStorageLayout::new(root);
    let legacy_wrapper = layout.legacy_model_wrapper_path(fixture.model_id);
    let wrapper_before = fs::read(&legacy_wrapper)?;
    let migrated_manifest = fixture.manifest.clone().migrate_legacy()?.canonicalized();
    let transaction_directory = layout
        .metadata_dir()
        .join(".transactions")
        .join("migration-model-interrupted");
    let plan = super::super::build_plan(root, &migrated_manifest, transaction_directory.clone())?;
    super::super::publish::write_journal(root, &migrated_manifest, &plan)?;
    super::super::failure_injection::set(4);
    assert!(super::super::publish::publish_plan(root, &migrated_manifest, &plan).is_err());
    assert!(!layout.canonical_manifest_path().exists());
    assert!(transaction_directory.is_dir());

    super::super::recover_interrupted_migration(root, Some(&migrated_manifest))?;

    assert_eq!(fs::read(&legacy_wrapper)?, wrapper_before);
    assert!(!transaction_directory.exists());
    Ok(())
}

#[test]
fn failed_rollback_preserves_transaction_for_a_later_attempt() -> Result<()> {
    let _guard = MIGRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = legacy_fixture();
    let root = fixture.directory.path();
    let layout = ProjectStorageLayout::new(root);
    let migrated_manifest = fixture.manifest.clone().migrate_legacy()?.canonicalized();
    let transaction_directory = layout
        .metadata_dir()
        .join(".transactions")
        .join("migration-rollback-failure");
    let plan = super::super::build_plan(root, &migrated_manifest, transaction_directory.clone())?;
    super::super::publish::write_journal(root, &migrated_manifest, &plan)?;
    super::super::failure_injection::set(4);
    assert!(super::super::publish::publish_plan(root, &migrated_manifest, &plan).is_err());

    let legacy_model_wrapper = layout.legacy_model_wrapper_path(fixture.model_id);
    let legacy_model_dir = legacy_model_wrapper.parent().unwrap();
    fs::create_dir_all(legacy_model_dir)?;
    assert!(super::super::recover_interrupted_migration(root, Some(&migrated_manifest)).is_err());
    assert!(transaction_directory.is_dir());
    assert!(
        transaction_directory
            .join(super::super::journal::JOURNAL_FILE)
            .is_file()
    );
    assert!(plan.models[0].backup_dir.is_dir());

    fs::remove_dir(legacy_model_dir)?;
    super::super::recover_interrupted_migration(root, Some(&migrated_manifest))?;
    assert!(!transaction_directory.exists());
    Ok(())
}

#[test]
fn committed_manifest_wins_over_a_stale_legacy_manifest_after_restart() -> Result<()> {
    let _guard = MIGRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = legacy_fixture();
    let root = fixture.directory.path();
    let layout = ProjectStorageLayout::new(root);
    let migrated_manifest = fixture.manifest.clone().migrate_legacy()?.canonicalized();
    let transaction_directory = layout
        .metadata_dir()
        .join(".transactions")
        .join("migration-committed");
    let plan = super::super::build_plan(root, &migrated_manifest, transaction_directory.clone())?;
    super::super::publish::write_journal(root, &migrated_manifest, &plan)?;
    super::super::failure_injection::set(3);
    assert!(super::super::publish::publish_plan(root, &migrated_manifest, &plan).is_err());
    assert!(layout.canonical_manifest_path().is_file());
    assert!(layout.legacy_manifest_path().is_file());

    let read = ManifestStore::read_validated(root)?;

    assert_eq!(read.raw(), &migrated_manifest);
    let canonical_model_wrapper = layout.canonical_model_wrapper_path(&migrated_manifest.models[0]);
    let canonical_model_dir = canonical_model_wrapper.parent().unwrap();
    assert!(
        !canonical_model_dir
            .join(super::super::LEGACY_MODEL_MARKER)
            .exists()
    );
    assert!(!layout.legacy_manifest_path().exists());
    assert!(!transaction_directory.exists());
    Ok(())
}

#[test]
fn invalid_canonical_manifest_preserves_recovery_transaction() -> Result<()> {
    let _guard = MIGRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = legacy_fixture();
    let root = fixture.directory.path();
    let layout = ProjectStorageLayout::new(root);
    let migrated_manifest = fixture.manifest.clone().migrate_legacy()?.canonicalized();
    let transaction_directory = layout
        .metadata_dir()
        .join(".transactions")
        .join("migration-invalid-canonical");
    let plan = super::super::build_plan(root, &migrated_manifest, transaction_directory.clone())?;
    super::super::publish::write_journal(root, &migrated_manifest, &plan)?;
    super::super::failure_injection::set(1);
    assert!(super::super::publish::publish_plan(root, &migrated_manifest, &plan).is_err());
    fs::write(layout.canonical_manifest_path(), b"not json")?;

    assert!(super::super::recover_interrupted_migration(root, Some(&migrated_manifest)).is_err());
    assert!(transaction_directory.is_dir());
    assert!(plan.scenes[0].backup_path.is_file());

    fs::remove_file(layout.canonical_manifest_path())?;
    super::super::recover_interrupted_migration(root, Some(&migrated_manifest))?;
    assert!(!transaction_directory.exists());
    Ok(())
}
