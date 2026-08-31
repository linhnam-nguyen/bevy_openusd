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
    super::super::publish::write_journal(&plan)?;
    super::super::failure_injection::set(4);
    assert!(super::super::publish::publish_plan(root, &migrated_manifest, &plan).is_err());
    assert!(!layout.canonical_manifest_path().exists());
    assert!(transaction_directory.is_dir());

    super::super::recover_interrupted_migration(root, Some(&migrated_manifest))?;

    assert_eq!(fs::read(&legacy_wrapper)?, wrapper_before);
    assert!(!transaction_directory.exists());
    Ok(())
}
