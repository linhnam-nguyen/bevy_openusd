use std::{fs, path::Path, path::PathBuf};

use anyhow::Result;
use tempfile::tempdir;

use super::*;
use crate::project::scene::authoring::author_scene_atomic;
use usd_project::SceneId;

fn write_fixture(directory: &Path, name: &str, text: &str) -> PathBuf {
    let path = directory.join(name);
    fs::write(&path, text).expect("write USD fixture");
    path
}

#[test]
fn native_usdhub_scene_is_classified_without_dependencies() -> Result<()> {
    let directory = tempdir()?;
    let scene_id = SceneId::new_v4();
    let path = author_scene_atomic(directory.path(), scene_id)?;

    let inspection = inspect_composition(&path)?;

    assert_eq!(
        inspection.classification,
        CompositionClassification::NativeUsdHubScene
    );
    assert!(!inspection.has_references);
    assert!(!inspection.has_payloads);
    assert!(inspection.dependencies.is_empty());
    Ok(())
}

#[test]
fn assembly_and_component_use_different_product_signals() -> Result<()> {
    let directory = tempdir()?;
    let assembly = write_fixture(
        directory.path(),
        "assembly.usda",
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
    );
    let component = write_fixture(
        directory.path(),
        "component.usda",
        r#"#usda 1.0
(
    defaultPrim = "Asset"
)
def Xform "Asset" (
    kind = "component"
) {}
"#,
    );

    assert_eq!(
        inspect_composition(&assembly)?.classification,
        CompositionClassification::SceneLike
    );
    assert_eq!(
        inspect_composition(&component)?.classification,
        CompositionClassification::ModelLike
    );
    Ok(())
}

#[test]
fn reference_and_payload_are_detected_without_loading_payloads() -> Result<()> {
    let directory = tempdir()?;
    let target = write_fixture(
        directory.path(),
        "target.usda",
        r#"#usda 1.0
(
    defaultPrim = "Asset"
)
def Xform "Asset" (
    kind = "component"
) {}
"#,
    );
    let target_name = target.file_name().unwrap().to_string_lossy();
    let source = write_fixture(
        directory.path(),
        "composed.usda",
        &format!(
            r#"#usda 1.0
(
    defaultPrim = "Assembly"
)
def Xform "Assembly" (
    kind = "assembly"
    references = @./{target_name}@</Asset>
) {{}}
"#
        ),
    );
    let payload_source = write_fixture(
        directory.path(),
        "payload.usda",
        &format!(
            r#"#usda 1.0
(
    defaultPrim = "Assembly"
)
def Xform "Assembly" (
    kind = "assembly"
    payload = @./{target_name}@</Asset>
) {{}}
"#
        ),
    );

    let inspection = inspect_composition(&source)?;

    assert_eq!(
        inspection.classification,
        CompositionClassification::SceneLike
    );
    assert!(inspection.has_references);
    assert!(
        inspection
            .dependencies
            .iter()
            .any(|dependency| dependency.classification == DependencyClassification::External)
    );
    let payload_inspection = inspect_composition(&payload_source)?;
    assert!(payload_inspection.has_payloads);
    assert!(!payload_inspection.has_references);
    Ok(())
}

#[test]
fn missing_reference_is_reported_as_missing_dependency() -> Result<()> {
    let directory = tempdir()?;
    let source = write_fixture(
        directory.path(),
        "missing.usda",
        r#"#usda 1.0
(
    defaultPrim = "Assembly"
)
def Xform "Assembly" (
    kind = "assembly"
    references = @./does-not-exist.usda@</Asset>
) {}
"#,
    );

    let inspection = inspect_composition(&source)?;

    assert_eq!(
        inspection.classification,
        CompositionClassification::SceneLike
    );
    assert!(inspection.has_references);
    assert!(
        inspection
            .dependencies
            .iter()
            .any(|dependency| dependency.classification == DependencyClassification::Missing)
    );
    assert!(
        inspection
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unresolved dependency"))
    );
    Ok(())
}

#[test]
fn plain_valid_usd_remains_ambiguous() -> Result<()> {
    let directory = tempdir()?;
    let source = write_fixture(
        directory.path(),
        "ambiguous.usda",
        r#"#usda 1.0
(
    defaultPrim = "Root"
)
def Xform "Root" {}
"#,
    );

    assert_eq!(
        inspect_composition(&source)?.classification,
        CompositionClassification::Ambiguous
    );
    Ok(())
}

#[test]
fn selected_variant_is_reported_as_composition_metadata() -> Result<()> {
    let directory = tempdir()?;
    let source = write_fixture(
        directory.path(),
        "variant.usda",
        r#"#usda 1.0
(
    defaultPrim = "Root"
)
def Xform "Root" (
    variants = { string lod = "high" }
    prepend variantSets = "lod"
) {
    variantSet "lod" = {
        "high" {
            def Xform "Detailed" {}
        }
    }
}
"#,
    );

    assert!(inspect_composition(&source)?.has_variants);
    Ok(())
}

#[test]
fn missing_default_prim_is_unsupported() -> Result<()> {
    let directory = tempdir()?;
    let source = write_fixture(
        directory.path(),
        "no-default.usda",
        "#usda 1.0\ndef Xform \"Root\" {}\n",
    );

    assert_eq!(
        inspect_composition(&source)?.classification,
        CompositionClassification::Unsupported
    );
    Ok(())
}
