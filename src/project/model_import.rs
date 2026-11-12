//! Backend-only Model importer ports and the USD capability implementation.
//!
//! Importers own source paths and OpenUSD work. Only the owned, source-neutral
//! inspection value is suitable for crossing into a later application route.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use usd_project::{CompositionClassification, CompositionInspection, ModelId, ModelSourceKind};

use super::scene::inspection::inspect_composition;

/// Source-neutral result of inspecting a possible Product Model source.
#[derive(Clone, Debug, PartialEq)]
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
    pub name: String,
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
        let name = request
            .source
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::trim)
            .filter(|stem| !stem.is_empty())
            .unwrap_or("Imported Model")
            .to_owned();
        Ok(PreparedModel {
            id: ModelId::new_v4(),
            name,
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
    pub(crate) const fn supported_extensions(&self) -> &'static [&'static str] {
        &["usd", "usda", "usdc", "usdz"]
    }

    pub(crate) const fn has_non_usd_importer(&self) -> bool {
        false
    }

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
#[path = "../../tests/unit/project/model_import_tests.rs"]
mod tests;
