//! Shared OpenUSD test utilities, synthetic USDA/stage generators, and stage fixtures.

use std::path::{Component, Path, PathBuf};

// Reuse the existing canonical fixture until BO-M7 moves its ownership.
pub const MODEL_USDA: &[u8] =
    include_bytes!("../../../crates/usd_semantic/tests/fixtures/identity_original.usda");

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
    stage_from_usda(include_str!("../../stages/live/reconcile_1.usda"))
        .expect("deep overlap stage opens")
}

/// Resolve a relative fixture only against the original workspace tests/stages root.
pub fn resolve_fixture_path(relative: &str) -> PathBuf {
    let relative = Path::new(relative);
    assert!(
        !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "fixture must be a relative path beneath tests/stages: {}",
        relative.display()
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("support crate is beneath tests/support")
        .join("stages");
    let path = root.join(relative);
    assert!(
        path.is_file(),
        "stage fixture does not exist: {}",
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
