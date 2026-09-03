//! Read-only USD asset inventory used to freeze the OR8 M2 fixtures.

use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

const SAMPLE_LIMIT: u64 = 1024 * 1024;

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
    let fingerprint = fingerprint(path)?;
    let scan = scan(path, &extension)?;
    let mut eligible = Vec::new();
    if scan.instance_indicator {
        eligible.push("A".to_owned());
    }
    if scan.animation || scan.dependency_count > 0 || scan.texture_references > 0 {
        eligible.push("B".to_owned());
    }
    if scan.bim_indicator {
        eligible.push("C".to_owned());
    }
    let asset_key = asset_key(origin, &relative_path);
    Ok(AssetRecord {
        asset_key,
        origin,
        relative_path,
        extension: extension.clone(),
        byte_size: metadata.len(),
        readable: File::open(path).is_ok(),
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
        fixture_eligibility: eligible,
        notes: scan.notes,
    })
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

fn fingerprint(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("hash {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[derive(Default)]
struct Scan {
    dependency_count: usize,
    texture_references: usize,
    animation: bool,
    instance_indicator: bool,
    bim_indicator: bool,
    arcs: Vec<String>,
    notes: String,
}

fn scan(path: &Path, extension: &str) -> Result<Scan, String> {
    let mut scan = Scan::default();
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().contains("pointinstanced"))
    {
        scan.instance_indicator = true;
    }
    if extension == "usdz" {
        let file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|error| format!("open USDZ {}: {error}", path.display()))?;
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|error| format!("read USDZ entry {}: {error}", path.display()))?;
            let name = entry.name().to_ascii_lowercase();
            scan_name(&name, &mut scan);
            if name.ends_with(".usd") || name.ends_with(".usda") || name.ends_with(".usdc") {
                let mut sample = Vec::new();
                entry
                    .take(SAMPLE_LIMIT)
                    .read_to_end(&mut sample)
                    .map_err(|error| format!("sample USDZ entry {}: {error}", path.display()))?;
                scan_bytes(&sample, &mut scan);
            }
        }
        scan.notes =
            "classified from USDZ central-directory entries and bounded USD samples".to_owned();
    } else {
        let mut file =
            File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
        let mut sample = Vec::new();
        file.take(SAMPLE_LIMIT)
            .read_to_end(&mut sample)
            .map_err(|error| format!("sample {}: {error}", path.display()))?;
        scan_bytes(&sample, &mut scan);
        scan.notes = "classified from bounded source sample; no geometry was loaded".to_owned();
    }
    scan.arcs.sort();
    scan.arcs.dedup();
    Ok(scan)
}

fn scan_name(name: &str, scan: &mut Scan) {
    if name.ends_with(".png") || name.ends_with(".jpg") || name.ends_with(".jpeg") {
        scan.texture_references += 1;
    }
    if name.contains("anim") || name.contains("walk") {
        scan.animation = true;
    }
}

fn scan_bytes(bytes: &[u8], scan: &mut Scan) {
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    for (marker, arc) in [
        ("references", "reference"),
        ("payload", "payload"),
        ("sublayers", "sublayer"),
    ] {
        let count = text.matches(marker).count();
        if count > 0 {
            scan.dependency_count += count;
            scan.arcs.push(arc.to_owned());
        }
    }
    scan.animation |=
        text.contains("anim") || text.contains("walk") || text.contains("timecodespersecond");
    scan.instance_indicator |= text.contains("pointinstancer") || text.contains("instanceable");
    scan.bim_indicator |=
        text.contains("revit") || text.contains("omniplugin") || text.contains("exported from");
    scan.texture_references += text.matches(".png").count() + text.matches(".jpg").count();
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
