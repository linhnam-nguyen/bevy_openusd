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
