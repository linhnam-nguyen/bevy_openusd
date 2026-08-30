//! Blocking, read-only inspection of composed USD import candidates.
//!
//! This adapter deliberately returns owned Project DTOs. OpenUSD stages and
//! filesystem paths stay inside the application/backend boundary; later UI and
//! worker routes can move the owned result without moving a stage handle.

use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result};
use openusd::{
    pcp,
    sdf::Value,
    usd::{InitialLoadSet, PrimPredicate, Stage},
};
use usd_project::{
    CompositionClassification, CompositionDiagnostic, CompositionInspection,
    DependencyClassification, DependencyInspection,
};

const MISSING_DEPENDENCY: &str = "<missing dependency>";
const UNSUPPORTED_DEPENDENCY: &str = "<unsupported dependency>";
const SCENE_ROOT_PRIM: &str = "SceneRoot";
const SCENE_ID_METADATA: &str = "usdhub:sceneId";
const SCHEMA_VERSION_METADATA: &str = "usdhub:schemaVersion";

/// Inspect a USD source without flattening it or loading payload geometry.
pub(crate) fn inspect_composition(source: &Path) -> Result<CompositionInspection> {
    let source_string = source
        .to_str()
        .context("USD inspection source path must be valid UTF-8")?;
    let stage = match Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(source_string)
    {
        Ok(stage) => stage,
        Err(error) => {
            return Ok(unsupported_inspection(format!(
                "unable to open source as USD: {}",
                summarize_error(&error)
            )));
        }
    };

    let mut inspection = CompositionInspection {
        classification: CompositionClassification::Ambiguous,
        dependencies: Vec::new(),
        diagnostics: Vec::new(),
        has_variants: false,
        has_payloads: false,
        has_references: false,
        has_sublayers: stage.layer_stack().len() > 1,
        spatial: crate::project::spatial::inspect_stage(&stage)
            .context("inspect USD source spatial convention")?,
    };
    if !inspection.spatial.up_axis_was_authored {
        inspection.diagnostics.push(diagnostic(
            "USD source does not author upAxis; OpenUSD fallback Y was used",
        ));
    }
    if !inspection.spatial.meters_per_unit_was_authored {
        inspection.diagnostics.push(diagnostic(
            "USD source does not author metersPerUnit; OpenUSD fallback 0.01 was used",
        ));
    }
    let mut dependencies = BTreeMap::new();

    collect_sublayer_dependencies(&stage, source, &mut dependencies);

    let Some(default_prim) = stage.default_prim() else {
        inspection.classification = CompositionClassification::Unsupported;
        inspection
            .diagnostics
            .push(diagnostic("USD source has no defaultPrim"));
        collect_composition_errors(&stage, &mut dependencies, &mut inspection);
        inspection.dependencies = finish_dependencies(dependencies);
        return Ok(inspection);
    };

    let root_path = format!("/{default_prim}");
    let root = stage.prim(root_path.as_str());
    if !root.is_defined()? {
        inspection.classification = CompositionClassification::Unsupported;
        inspection
            .diagnostics
            .push(diagnostic("USD source defaultPrim is not a defined prim"));
        collect_composition_errors(&stage, &mut dependencies, &mut inspection);
        inspection.dependencies = finish_dependencies(dependencies);
        return Ok(inspection);
    }

    let is_native_scene = native_usdhub_scene(&root)?;
    let root_is_group = root.is_group()?;
    let root_is_component = root.is_component()?;

    stage.traverse(PrimPredicate::ALL, |path| {
        let prim = stage.prim(path.clone());
        for identifier in stage.layer_identifiers() {
            if let Some(layer) = stage.layer(&identifier)
                && let Some(spec) = layer.prim(path.clone())
            {
                inspection.has_references |= spec.has_field("references");
                inspection.has_payloads |= spec.has_field("payload");
            }
        }
        match prim.variant_sets().get_all_variant_selections() {
            Ok(selections) => inspection.has_variants |= !selections.is_empty(),
            Err(_) => inspection
                .diagnostics
                .push(diagnostic("unable to inspect one prim's variant metadata")),
        }

        let Ok(index) = prim.prim_index().graph() else {
            inspection
                .diagnostics
                .push(diagnostic("unable to inspect one prim's composition graph"));
            return;
        };
        for node in index.all_nodes() {
            match node.arc() {
                pcp::ArcType::Reference => {
                    inspection.has_references = true;
                    if let Some(identifier) = stage.layer_identifier(node.layer_id()) {
                        record_dependency(&mut dependencies, identifier, source);
                    }
                }
                pcp::ArcType::Payload => {
                    inspection.has_payloads = true;
                    if let Some(identifier) = stage.layer_identifier(node.layer_id()) {
                        record_dependency(&mut dependencies, identifier, source);
                    }
                }
                _ => {}
            }
        }
    })?;

    collect_composition_errors(&stage, &mut dependencies, &mut inspection);
    inspection.classification = classify(
        is_native_scene,
        root_is_group,
        root_is_component,
        inspection.has_references,
        inspection.has_payloads,
        inspection.has_sublayers,
        inspection.has_variants,
    );
    inspection.dependencies = finish_dependencies(dependencies);
    Ok(inspection)
}

fn classify(
    is_native_scene: bool,
    root_is_group: bool,
    root_is_component: bool,
    has_references: bool,
    has_payloads: bool,
    has_sublayers: bool,
    has_variants: bool,
) -> CompositionClassification {
    if is_native_scene {
        return CompositionClassification::NativeUsdHubScene;
    }
    if root_is_component {
        return CompositionClassification::ModelLike;
    }
    if root_is_group || has_sublayers || (has_variants && (has_references || has_payloads)) {
        return CompositionClassification::SceneLike;
    }
    CompositionClassification::Ambiguous
}

fn native_usdhub_scene(root: &openusd::usd::Prim) -> Result<bool> {
    let Some(Value::Dictionary(data)) = root.custom_data()? else {
        return Ok(false);
    };
    let scene_id_is_string = data
        .get(SCENE_ID_METADATA)
        .and_then(Value::as_str)
        .is_some();
    let schema_is_current = data
        .get(SCHEMA_VERSION_METADATA)
        .is_some_and(|value| matches!(value, Value::Int(1 | 2)));
    Ok(root.path().as_str() == format!("/{SCENE_ROOT_PRIM}")
        && scene_id_is_string
        && schema_is_current)
}

fn collect_sublayer_dependencies(
    stage: &Stage,
    source: &Path,
    dependencies: &mut BTreeMap<String, DependencyClassification>,
) {
    for identifier in stage.layer_stack().into_iter().skip(1) {
        record_dependency(dependencies, identifier, source);
    }
}

fn collect_composition_errors(
    stage: &Stage,
    dependencies: &mut BTreeMap<String, DependencyClassification>,
    inspection: &mut CompositionInspection,
) {
    for error in stage.composition_errors() {
        let error_text = error.to_string();
        inspection.has_references |= error_text.contains("Reference");
        inspection.has_payloads |= error_text.contains("Payload");
        match error {
            pcp::Error::UnresolvedLayer { .. }
            | pcp::Error::UnresolvedPrimPath { .. }
            | pcp::Error::MissingDefaultPrim { .. } => {
                dependencies
                    .entry(MISSING_DEPENDENCY.to_owned())
                    .or_insert(DependencyClassification::Missing);
                inspection.diagnostics.push(diagnostic(
                    "USD composition contains an unresolved dependency",
                ));
            }
            pcp::Error::MalformedLayer { .. }
            | pcp::Error::InvalidPrimPath { .. }
            | pcp::Error::InvalidDefaultPrim { .. } => {
                dependencies
                    .entry(UNSUPPORTED_DEPENDENCY.to_owned())
                    .or_insert(DependencyClassification::Unsupported);
                inspection.diagnostics.push(diagnostic(
                    "USD composition contains an unsupported dependency or arc",
                ));
            }
            _ => inspection.diagnostics.push(diagnostic(
                "USD composition reported a composition diagnostic",
            )),
        }
    }
}

fn record_dependency(
    dependencies: &mut BTreeMap<String, DependencyClassification>,
    identifier: String,
    source: &Path,
) {
    let classification = classify_dependency(&identifier, source);
    dependencies
        .entry(identifier)
        .and_modify(|current| *current = merge_classification(*current, classification))
        .or_insert(classification);
}

fn classify_dependency(identifier: &str, source: &Path) -> DependencyClassification {
    if identifier.starts_with("anon:") {
        return DependencyClassification::Unsupported;
    }
    let identifier_path = Path::new(identifier);
    if identifier_path
        .components()
        .any(|component| component.as_os_str() == ".usdhub")
        || source
            .parent()
            .is_some_and(|parent| identifier_path.starts_with(parent.join(".usdhub")))
    {
        DependencyClassification::ProjectControlled
    } else {
        DependencyClassification::External
    }
}

fn merge_classification(
    current: DependencyClassification,
    next: DependencyClassification,
) -> DependencyClassification {
    use DependencyClassification::*;
    match (current, next) {
        (Missing, _) | (_, Missing) => Missing,
        (Unsupported, _) | (_, Unsupported) => Unsupported,
        (ProjectControlled, ProjectControlled) => ProjectControlled,
        _ => External,
    }
}

fn finish_dependencies(
    dependencies: BTreeMap<String, DependencyClassification>,
) -> Vec<DependencyInspection> {
    dependencies
        .into_iter()
        .map(|(identifier, classification)| DependencyInspection {
            identifier: dependency_label(&identifier, classification),
            classification,
        })
        .collect()
}

fn dependency_label(identifier: &str, classification: DependencyClassification) -> String {
    if identifier.starts_with('<') || identifier.starts_with("anon:") {
        return identifier.to_owned();
    }
    match classification {
        DependencyClassification::ProjectControlled => {
            let mut components = Path::new(identifier).components();
            let Some(_) = components.find(|component| component.as_os_str() == ".usdhub") else {
                return "project:controlled-dependency".to_owned();
            };
            let suffix = components
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            if suffix.is_empty() {
                "project:.usdhub".to_owned()
            } else {
                format!("project:.usdhub/{suffix}")
            }
        }
        DependencyClassification::External => Path::new(identifier)
            .file_name()
            .and_then(|name| name.to_str())
            .map_or_else(
                || "external:dependency".to_owned(),
                |name| format!("external:{name}"),
            ),
        DependencyClassification::Missing => "<missing dependency>".to_owned(),
        DependencyClassification::Unsupported => "<unsupported dependency>".to_owned(),
    }
}

fn diagnostic(message: impl Into<String>) -> CompositionDiagnostic {
    CompositionDiagnostic {
        message: message.into(),
    }
}

fn unsupported_inspection(message: String) -> CompositionInspection {
    CompositionInspection {
        classification: CompositionClassification::Unsupported,
        dependencies: vec![],
        diagnostics: vec![diagnostic(message)],
        has_variants: false,
        has_payloads: false,
        has_references: false,
        has_sublayers: false,
        spatial: Default::default(),
    }
}

fn summarize_error(error: &anyhow::Error) -> String {
    error.root_cause().to_string().split_once(':').map_or_else(
        || "source could not be opened".to_owned(),
        |(kind, _)| kind.to_owned(),
    )
}

#[cfg(test)]
#[path = "inspection_tests.rs"]
mod inspection_tests;
