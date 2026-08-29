//! Source-neutral results from inspecting a composed USD source.

use serde::{Deserialize, Serialize};

use crate::SourceSpatialConvention;

/// Product-level meaning suggested by a composed USD source.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum CompositionClassification {
    /// A canonical USDHub Scene authored by the Project Scene adapter.
    NativeUsdHubScene,
    /// A composed assembly with enough hierarchy/composition evidence to offer
    /// Scene adoption, but without native USDHub identity metadata.
    SceneLike,
    /// A reusable authored asset that should be adopted as one opaque Model.
    ModelLike,
    /// The source is valid USD, but its product meaning needs an explicit user
    /// choice before adoption.
    Ambiguous,
    /// The source cannot be safely inspected as an adoption candidate.
    Unsupported,
}

/// Ownership/readability classification for one composed dependency.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DependencyClassification {
    /// Resides in USDHub-controlled Project storage.
    ProjectControlled,
    /// Resolved successfully but remains outside Project-controlled storage.
    External,
    /// Authored or required by composition but could not be resolved.
    Missing,
    /// Present in the source graph but not supported by the inspection route.
    Unsupported,
}

/// One owned dependency observation. The identifier is an opaque, sanitized
/// label; no OpenUSD handle or filesystem path crosses this contract.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DependencyInspection {
    pub identifier: String,
    pub classification: DependencyClassification,
}

/// A stable, user-safe diagnostic produced during inspection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompositionDiagnostic {
    pub message: String,
}

/// Immutable inspection result used by later Scene adoption and Model import
/// flows. It intentionally contains no Stage, renderer, Git, or filesystem
/// objects.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CompositionInspection {
    pub classification: CompositionClassification,
    pub dependencies: Vec<DependencyInspection>,
    pub diagnostics: Vec<CompositionDiagnostic>,
    pub has_variants: bool,
    pub has_payloads: bool,
    pub has_references: bool,
    pub has_sublayers: bool,
    pub spatial: SourceSpatialConvention,
}
