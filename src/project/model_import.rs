//! Backend-only Model importer ports and the USD capability implementation.
//!
//! Importers own source paths and OpenUSD work. Only the owned, source-neutral
//! inspection value is suitable for crossing into a later application route.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use usd_project::{CompositionClassification, CompositionInspection, ModelId, ModelSourceKind};

use super::scene::inspection::inspect_composition;

/// Source-neutral result of inspecting a possible Product Model source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModelImportInspection {
    pub composition: CompositionInspection,
}

/// Backend-owned input to importer preparation.
pub(crate) struct ModelImportRequest {
    pub source: PathBuf,
    pub inspection: ModelImportInspection,
}

/// A prepared opaque Model identity, before a storage wrapper is published.
#[derive(Clone, Debug)]
pub(crate) struct PreparedModel {
    pub id: ModelId,
    pub source_kind: ModelSourceKind,
    pub source: PathBuf,
    pub inspection: ModelImportInspection,
}

/// Port implemented by one source-specific Model importer.
pub(crate) trait ModelImporter {
    fn kind(&self) -> ModelSourceKind;
    fn inspect(&self, source: &Path) -> Result<ModelImportInspection>;
    fn prepare(&self, request: ModelImportRequest) -> Result<PreparedModel>;
}

/// USD importer for opaque Model adoption.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct UsdModelImporter;

impl ModelImporter for UsdModelImporter {
    fn kind(&self) -> ModelSourceKind {
        ModelSourceKind::Usd
    }

    fn inspect(&self, source: &Path) -> Result<ModelImportInspection> {
        let composition = inspect_composition(source).context("inspect USD Model source")?;
        ensure_model_importable(&composition)?;
        Ok(ModelImportInspection { composition })
    }

    fn prepare(&self, request: ModelImportRequest) -> Result<PreparedModel> {
        ensure!(
            request.source.is_file(),
            "USD Model source disappeared or is not a file"
        );
        ensure_model_importable(&request.inspection.composition)?;
        let revalidated = self.inspect(&request.source)?;
        ensure!(
            revalidated == request.inspection,
            "USD Model source changed after inspection"
        );
        Ok(PreparedModel {
            id: ModelId::new_v4(),
            source_kind: self.kind(),
            source: request.source,
            inspection: revalidated,
        })
    }
}

/// Version-one importer registry. USD is the only capability implemented.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ModelImporterRegistry {
    usd: UsdModelImporter,
}

impl ModelImporterRegistry {
    pub(crate) fn importer_for(&self, kind: &ModelSourceKind) -> Option<&dyn ModelImporter> {
        match kind {
            ModelSourceKind::Usd => Some(&self.usd),
            ModelSourceKind::External(_) => None,
        }
    }
}

fn ensure_model_importable(inspection: &CompositionInspection) -> Result<()> {
    ensure!(
        !matches!(
            inspection.classification,
            CompositionClassification::Unsupported
        ),
        "USD source is not a supported opaque Model candidate"
    );
    ensure!(
        inspection.dependencies.iter().all(|dependency| {
            !matches!(
                dependency.classification,
                usd_project::DependencyClassification::Missing
                    | usd_project::DependencyClassification::Unsupported
            )
        }),
        "USD Model source has unresolved or unsupported dependencies"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
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
}
