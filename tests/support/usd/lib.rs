//! Shared OpenUSD test utilities, synthetic USDA/stage generators, and stage fixtures.

use std::path::{Component, Path, PathBuf};

pub const MODEL_USDA: &[u8] = include_bytes!("../../fixtures/usd/identity_original.usda");

pub fn synthetic_wide_usda() -> String {
    let mut usda = String::from("#usda 1.0\n\ndef Xform \"World\"\n{\n");
    for group in ["A", "B", "C"] {
        usda.push_str(&format!("    def Xform \"{group}\"\n    {{\n"));
        for i in 0..10 {
            usda.push_str(&format!(
                "        def Xform \"{group}{i}\"\n        {{\n        }}\n"
            ));
        }
        usda.push_str("    }\n");
    }
    usda.push_str("}\n");
    usda
}

/// Open a standalone snippet using an isolated temporary directory.
pub fn stage_from_usda(usda_text: &str) -> anyhow::Result<openusd::usd::Stage> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("stage.usda");
    std::fs::write(&path, usda_text)?;
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("temp path is not valid UTF-8"))?;
    // As with UsdSnippet, the stage owns the parsed data after opening.
    openusd::usd::Stage::open(path_str)
}

pub fn make_synthetic_wide_stage() -> openusd::usd::Stage {
    stage_from_usda(&synthetic_wide_usda()).expect("synthetic wide stage opens")
}

pub fn make_deep_overlap_stage() -> openusd::usd::Stage {
    stage_from_usda(include_str!("../../fixtures/usd/reconcile_1.usda"))
        .expect("deep overlap stage opens")
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("usd_support crate is beneath tests/support/usd")
}

fn validate_relative_path<'a>(relative: &'a str, domain: &str) -> &'a Path {
    let path = Path::new(relative);
    assert!(
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "{domain} must be a relative path with normal components only: {}",
        path.display()
    );
    path
}

/// Return the normalized shared USD fixture directory (`tests/fixtures/usd`).
pub fn usd_fixtures_dir() -> PathBuf {
    workspace_root().join("tests").join("fixtures").join("usd")
}

/// Return the shared USD fixture directory formatted for Bevy's `AssetPlugin::file_path`.
pub fn usd_fixtures_asset_root() -> String {
    usd_fixtures_dir().to_string_lossy().into_owned()
}

/// Resolve a shared USD fixture path beneath `tests/fixtures/usd` without checking existence.
pub fn resolve_fixture_path_unchecked(relative: &str) -> PathBuf {
    let relative = validate_relative_path(relative, "fixture");
    usd_fixtures_dir().join(relative)
}

/// Resolve a relative shared USD fixture against `tests/fixtures/usd`, asserting it exists.
pub fn resolve_fixture_path(relative: &str) -> PathBuf {
    let path = resolve_fixture_path_unchecked(relative);
    assert!(
        path.is_file(),
        "USD fixture does not exist: {}",
        path.display()
    );
    path
}

/// Resolve a workspace runtime/example asset path beneath `assets/`, asserting it exists.
pub fn resolve_asset_path(relative: &str) -> PathBuf {
    let relative = validate_relative_path(relative, "asset");
    let path = workspace_root().join("assets").join(relative);
    assert!(
        path.is_file(),
        "asset file does not exist: {}",
        path.display()
    );
    path
}

/// Resolve an ignored real-asset path beneath `../external_assets/` (sibling of the workspace).
pub fn resolve_external_asset_path(relative: &str) -> PathBuf {
    let relative = validate_relative_path(relative, "external asset");
    workspace_root()
        .parent()
        .expect("workspace has parent directory for external_assets")
        .join("external_assets")
        .join(relative)
}

/// Resolve a crate-local fixture beneath `<manifest_dir>/tests/fixtures/`, asserting it exists.
pub fn resolve_crate_fixture_path(manifest_dir: &str, relative: &str) -> PathBuf {
    let relative = validate_relative_path(relative, "crate fixture");
    let path = Path::new(manifest_dir)
        .join("tests")
        .join("fixtures")
        .join(relative);
    assert!(
        path.is_file(),
        "crate fixture does not exist: {}",
        path.display()
    );
    path
}

pub fn open_fixture_stage(relative: &str) -> openusd::usd::Stage {
    let path = resolve_fixture_path(relative);
    let path_str = path.to_str().expect("fixture path is valid utf-8");
    openusd::usd::Stage::open(path_str)
        .unwrap_or_else(|err| panic!("failed to open fixture stage at {}: {err}", path.display()))
}

#[cfg(test)]
#[path = "tests/unit/self_tests.rs"]
mod tests;
