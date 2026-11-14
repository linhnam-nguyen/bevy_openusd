//! Read-only USD asset inventory used to freeze the OR8 M2 fixtures.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::asset_inspection;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AssetOrigin {
    BevyOpenUsd,
    ExternalAssets,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct AssetRecord {
    pub asset_key: String,
    pub origin: AssetOrigin,
    pub relative_path: String,
    pub extension: String,
    pub byte_size: u64,
    pub readable: bool,
    pub usd_layer_or_package_type: String,
    pub dependency_count: usize,
    pub external_texture_references: usize,
    pub animation_presence: bool,
    pub point_instancer_or_native_instance_indicators: bool,
    pub bim_revit_semantic_indicators: bool,
    pub composition_arcs: Vec<String>,
    pub stable_content_fingerprint: String,
    pub fixture_eligibility: Vec<String>,
    pub notes: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct AssetDictionary {
    pub schema_version: u32,
    pub assets: Vec<AssetRecord>,
}

#[derive(Clone, Debug)]
pub(super) struct CanonicalFixtures {
    pub instance_heavy: AssetRecord,
    pub dependency_animation: AssetRecord,
    pub bim_revit: AssetRecord,
    pub instance_path: PathBuf,
    pub dependency_animation_path: PathBuf,
    pub bim_revit_path: PathBuf,
}

pub(super) fn inventory(
    bevy_assets_root: &Path,
    external_assets_root: &Path,
) -> Result<AssetDictionary, String> {
    let mut assets = Vec::new();
    collect_assets(
        bevy_assets_root,
        AssetOrigin::BevyOpenUsd,
        bevy_assets_root,
        &mut assets,
    )?;
    collect_assets(
        external_assets_root,
        AssetOrigin::ExternalAssets,
        external_assets_root,
        &mut assets,
    )?;
    assets.sort_by(|left, right| left.asset_key.cmp(&right.asset_key));
    Ok(AssetDictionary {
        schema_version: 1,
        assets,
    })
}

pub(super) fn resolve_fixtures(
    dictionary: &AssetDictionary,
    bevy_assets_root: &Path,
    external_assets_root: &Path,
) -> Result<CanonicalFixtures, String> {
    let candidates = [
        (
            "A",
            AssetOrigin::BevyOpenUsd,
            "external/PointInstancedMedCity.usdz",
        ),
        ("B", AssetOrigin::BevyOpenUsd, "external/HumanFemale.usdz"),
        (
            "C",
            AssetOrigin::ExternalAssets,
            "Omniverse/V1/Projet1.usdc",
        ),
    ];
    let mut records = candidates.into_iter().map(|(fixture, origin, path)| {
        let key = asset_key(origin, path);
        let record = dictionary
            .assets
            .iter()
            .find(|asset| asset.asset_key == key)
            .cloned()
            .ok_or_else(|| format!("canonical fixture {fixture} is not in asset dictionary"))?;
        if !record
            .fixture_eligibility
            .iter()
            .any(|value| value == fixture)
        {
            return Err(format!(
                "canonical fixture {fixture} is not eligible according to the dictionary"
            ));
        }
        let root = match origin {
            AssetOrigin::BevyOpenUsd => bevy_assets_root,
            AssetOrigin::ExternalAssets => external_assets_root,
        };
        let absolute = root.join(path);
        if !absolute.is_file() {
            return Err(format!(
                "canonical fixture {fixture} is missing: {}",
                absolute.display()
            ));
        }
        Ok((record, absolute))
    });
    let (instance_heavy, instance_path) = records.next().expect("fixture A")?;
    let (dependency_animation, dependency_animation_path) = records.next().expect("fixture B")?;
    let (bim_revit, bim_revit_path) = records.next().expect("fixture C")?;
    Ok(CanonicalFixtures {
        instance_heavy,
        dependency_animation,
        bim_revit,
        instance_path,
        dependency_animation_path,
        bim_revit_path,
    })
}

fn collect_assets(
    root: &Path,
    origin: AssetOrigin,
    current: &Path,
    assets: &mut Vec<AssetRecord>,
) -> Result<(), String> {
    if !root.is_dir() {
        return Err(format!("asset root is not a directory: {}", root.display()));
    }
    let mut entries = fs::read_dir(current)
        .map_err(|error| format!("read asset directory {}: {error}", current.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read asset entry {}: {error}", current.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_assets(root, origin, &path, assets)?;
        } else if is_usd_asset(&path) {
            assets.push(inventory_file(root, origin, &path)?);
        }
    }
    Ok(())
}

fn inventory_file(root: &Path, origin: AssetOrigin, path: &Path) -> Result<AssetRecord, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("stat {}: {error}", path.display()))?;
    let relative_path = path
        .strip_prefix(root)
        .map_err(|_| format!("asset escaped inventory root: {}", path.display()))?
        .to_string_lossy()
        .replace('\\', "/");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let asset_key = asset_key(origin, &relative_path);
    let authoritative_fixture = canonical_fixture_key(origin, &relative_path);
    let fingerprint = asset_inspection::fingerprint(path)?;
    let scan = asset_inspection::inspect(path, &extension, authoritative_fixture.is_some())?;
    let fixture_eligibility = authoritative_fixture
        .filter(|fixture| actual_fixture_signal(fixture, &scan))
        .map(|fixture| vec![fixture.to_owned()])
        .unwrap_or_default();
    Ok(AssetRecord {
        asset_key,
        origin,
        relative_path,
        extension: extension.clone(),
        byte_size: metadata.len(),
        readable: fs::File::open(path).is_ok(),
        usd_layer_or_package_type: match extension.as_str() {
            "usdz" => "usdz_archive".to_owned(),
            "usdc" => "binary_usd_layer".to_owned(),
            _ => "text_usd_layer".to_owned(),
        },
        dependency_count: scan.dependency_count,
        external_texture_references: scan.texture_references,
        animation_presence: scan.animation,
        point_instancer_or_native_instance_indicators: scan.instance_indicator,
        bim_revit_semantic_indicators: scan.bim_indicator,
        composition_arcs: scan.arcs,
        stable_content_fingerprint: fingerprint,
        fixture_eligibility,
        notes: scan.notes,
    })
}

fn canonical_fixture_key(origin: AssetOrigin, relative_path: &str) -> Option<&'static str> {
    match (origin, relative_path) {
        (AssetOrigin::BevyOpenUsd, "external/PointInstancedMedCity.usdz") => Some("A"),
        (AssetOrigin::BevyOpenUsd, "external/HumanFemale.usdz") => Some("B"),
        (AssetOrigin::ExternalAssets, "Omniverse/V1/Projet1.usdc") => Some("C"),
        _ => None,
    }
}

fn actual_fixture_signal(fixture: &str, scan: &asset_inspection::AssetScan) -> bool {
    match fixture {
        "A" => scan.instance_indicator,
        "B" => scan.animation,
        "C" => scan.bim_indicator,
        _ => false,
    }
}

fn asset_key(origin: AssetOrigin, relative_path: &str) -> String {
    let prefix = match origin {
        AssetOrigin::BevyOpenUsd => "bevy_openusd",
        AssetOrigin::ExternalAssets => "external_assets",
    };
    format!("{prefix}:{relative_path}")
}

fn is_usd_asset(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "usd" | "usda" | "usdc" | "usdz"
            )
        })
}

pub(super) fn default_roots() -> (PathBuf, PathBuf) {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let bevy_assets = repository_root.join("assets");
    let external_assets = repository_root
        .parent()
        .unwrap_or(repository_root)
        .join("Instance2/external_assets");
    (bevy_assets, external_assets)
}
