use std::fs;

use tempfile::tempdir;

use super::*;

fn model_source(directory: &Path, name: &str) -> PathBuf {
    let path = directory.join(name);
    fs::write(
        &path,
        r#"#usda 1.0
(
defaultPrim = "Asset"
)
def Xform "Asset" (
kind = "component"
) {}
"#,
    )
    .unwrap();
    path
}

#[test]
fn usd_importer_reports_model_like_sources() -> Result<()> {
    let directory = tempdir()?;
    let source = model_source(directory.path(), "asset.usda");
    let importer = UsdModelImporter;

    let inspection = importer.inspect(&source)?;

    assert_eq!(importer.kind(), ModelSourceKind::Usd);
    assert_eq!(
        inspection.composition.classification,
        CompositionClassification::ModelLike
    );
    Ok(())
}

#[test]
fn registry_exposes_only_current_backend_capability() {
    let registry = ModelImporterRegistry::default();

    assert_eq!(
        registry.supported_extensions(),
        &["usd", "usda", "usdc", "usdz"]
    );
    assert!(!registry.has_non_usd_importer());
    assert!(
        registry
            .importer_for(&ModelSourceKind::External("glb".to_owned()))
            .is_none()
    );
}

#[test]
fn scene_like_source_can_be_prepared_as_one_opaque_model() -> Result<()> {
    let directory = tempdir()?;
    let source = directory.path().join("assembly.usda");
    fs::write(
        &source,
        r#"#usda 1.0
(
defaultPrim = "Assembly"
)
def Xform "Assembly" (
kind = "assembly"
) {
def Xform "Child" {}
}
"#,
    )?;
    let importer = UsdModelImporter;
    let inspection = importer.inspect(&source)?;

    assert_eq!(
        inspection.composition.classification,
        CompositionClassification::SceneLike
    );
    let prepared = importer.prepare(ModelImportRequest { source, inspection })?;

    assert_eq!(prepared.source_kind, ModelSourceKind::Usd);
    assert_eq!(prepared.name, "assembly");
    assert!(!prepared.id.as_uuid().is_nil());
    Ok(())
}

#[test]
fn prepare_allocates_distinct_opaque_model_identities() -> Result<()> {
    let directory = tempdir()?;
    let source = model_source(directory.path(), "asset.usda");
    let importer = UsdModelImporter;
    let inspection = importer.inspect(&source)?;

    let first = importer.prepare(ModelImportRequest {
        source: source.clone(),
        inspection: inspection.clone(),
    })?;
    let second = importer.prepare(ModelImportRequest {
        source: source.clone(),
        inspection,
    })?;

    assert_ne!(first.id, second.id);
    assert_eq!(first.source_kind, ModelSourceKind::Usd);
    assert_eq!(first.source, source);
    assert_eq!(first.name, "asset");
    Ok(())
}

#[test]
fn stale_inspection_is_rejected_before_preparation() -> Result<()> {
    let directory = tempdir()?;
    let source = model_source(directory.path(), "asset.usda");
    let importer = UsdModelImporter;
    let inspection = importer.inspect(&source)?;
    fs::write(
        &source,
        r#"#usda 1.0
(
defaultPrim = "Assembly"
)
def Xform "Assembly" (kind = "assembly") {}
"#,
    )?;

    assert!(
        importer
            .prepare(ModelImportRequest { source, inspection })
            .is_err()
    );
    Ok(())
}

#[test]
fn registry_has_no_unimplemented_external_importer() {
    let registry = ModelImporterRegistry::default();
    assert!(registry.importer_for(&ModelSourceKind::Usd).is_some());
    assert!(
        registry
            .importer_for(&ModelSourceKind::External("ifc".to_owned()))
            .is_none()
    );
}
