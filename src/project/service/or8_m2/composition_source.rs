//! Run-owned resolver support for the canonical external BIM fixture.

use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) fn prepare_bim_link_source(source: &Path, directory: &Path) -> Result<PathBuf, String> {
    let original_directory = source
        .parent()
        .ok_or_else(|| format!("BIM fixture has no parent: {}", source.display()))?;
    let file_name = source
        .file_name()
        .ok_or_else(|| format!("BIM fixture has no filename: {}", source.display()))?;
    let mirror = directory.join("fixture-C-link-source");
    fs::create_dir_all(&mirror).map_err(|error| format!("create C link source mirror: {error}"))?;
    for name in ["Projet1.usdc", "Looks.usdc"] {
        fs::copy(original_directory.join(name), mirror.join(name))
            .map_err(|error| format!("copy C fixture layer {name}: {error}"))?;
    }
    for name in ["OmniGlass.mdl", "OmniPBR.mdl"] {
        fs::write(
            mirror.join(name),
            b"# OR8 M2 resolver support placeholder; source bytes remain unchanged.\n",
        )
        .map_err(|error| format!("write C fixture resolver support {name}: {error}"))?;
    }
    for relative in [
        "Looks/Béton, coulé sur place/normal.png",
        "Looks/Châssis/albedo.png",
        "Looks/Porte - Panneau/albedo.png",
        "Looks/Site - Agrégats/albedo.png",
        "Looks/Site - Agrégats/normal.png",
    ] {
        let target = mirror.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create C fixture texture support: {error}"))?;
        }
        fs::write(
            target,
            b"OR8 M2 resolver support placeholder; source bytes remain unchanged.\n",
        )
        .map_err(|error| format!("write C fixture texture support {relative}: {error}"))?;
    }
    Ok(mirror.join(file_name))
}
