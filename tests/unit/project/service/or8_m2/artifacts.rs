//! Controlled OR8 M2 TestSpaces artifact locations.

use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use super::assets::AssetDictionary;

pub(super) fn m2_testspaces_root() -> PathBuf {
    std::env::var_os("USDHUB_OR8_M2_ROOT").map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
                .join("TestSpaces/OR8/M2")
        },
        PathBuf::from,
    )
}

/// Start one deterministic test run from a clean, run-owned directory.
///
/// Only the exact child named by the harness is removed. This keeps reruns
/// deterministic without consuming artifacts from another checkpoint.
pub(super) fn clean_run_directory(name: &str) -> Result<PathBuf, String> {
    validate_component(name, "run")?;
    let runs_root = m2_testspaces_root().join("runs");
    fs::create_dir_all(&runs_root)
        .map_err(|error| format!("create {}: {error}", runs_root.display()))?;
    let run_directory = runs_root.join(name);
    if run_directory.exists() {
        fs::remove_dir_all(&run_directory)
            .map_err(|error| format!("clean {}: {error}", run_directory.display()))?;
    }
    fs::create_dir(&run_directory)
        .map_err(|error| format!("create {}: {error}", run_directory.display()))?;
    Ok(run_directory)
}

pub(super) fn clean_output_directory(group: &str, name: &str) -> Result<PathBuf, String> {
    validate_component(group, "output group")?;
    validate_component(name, "output")?;
    let group_directory = m2_testspaces_root().join(group);
    fs::create_dir_all(&group_directory)
        .map_err(|error| format!("create {}: {error}", group_directory.display()))?;
    let output_directory = group_directory.join(name);
    if output_directory.exists() {
        fs::remove_dir_all(&output_directory)
            .map_err(|error| format!("clean {}: {error}", output_directory.display()))?;
    }
    fs::create_dir(&output_directory)
        .map_err(|error| format!("create {}: {error}", output_directory.display()))?;
    Ok(output_directory)
}

fn validate_component(value: &str, kind: &str) -> Result<(), String> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
    {
        return Err(format!("invalid OR8/M2 {kind} name: {value:?}"));
    }
    Ok(())
}

pub(super) fn write_dictionary(root: &Path, dictionary: &AssetDictionary) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|error| format!("create {}: {error}", root.display()))?;
    let target = root.join("asset_dictionary.json");
    let temporary = root.join("asset_dictionary.json.tmp");
    let bytes = serde_json::to_vec_pretty(dictionary).map_err(|error| error.to_string())?;
    let mut file = File::create(&temporary)
        .map_err(|error| format!("create {}: {error}", temporary.display()))?;
    file.write_all(&bytes)
        .map_err(|error| format!("write {}: {error}", temporary.display()))?;
    file.sync_all()
        .map_err(|error| format!("sync {}: {error}", temporary.display()))?;
    fs::rename(&temporary, &target)
        .map_err(|error| format!("publish {}: {error}", target.display()))
}
