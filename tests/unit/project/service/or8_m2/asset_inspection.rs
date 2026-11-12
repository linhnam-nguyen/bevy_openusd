//! OpenUSD-backed inspection for the small set of canonical M2 asset fixtures.
//!
//! The complete inventory still records cheap package metadata for every USD
//! file.  Only the three canonical candidates are authoritative for fixture
//! eligibility, and those candidates are classified from an opened Stage and
//! semantic extraction rather than from names or bounded text markers.

use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};

use openusd::usd::{InitialLoadSet, PrimPredicate, Stage};
use usd_model::SnapshotSource;
use usd_semantic::{SemanticConfig, SemanticExtractor};

const SAMPLE_LIMIT: u64 = 1024 * 1024;

#[derive(Default)]
pub(crate) struct AssetScan {
    pub(crate) dependency_count: usize,
    pub(crate) texture_references: usize,
    pub(crate) animation: bool,
    pub(crate) instance_indicator: bool,
    pub(crate) bim_indicator: bool,
    pub(crate) arcs: Vec<String>,
    pub(crate) notes: String,
}

pub(crate) fn fingerprint(path: &Path) -> Result<String, String> {
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

pub(crate) fn inspect(
    path: &Path,
    extension: &str,
    authoritative: bool,
) -> Result<AssetScan, String> {
    let mut scan = AssetScan::default();
    if extension == "usdz" {
        inspect_package_entries(path, &mut scan)?;
    }
    if authoritative {
        inspect_openusd_stage(path, &mut scan)?;
        scan.notes = format!(
            "classified from OpenUSD Stage composition and semantic extraction; package_textures={}",
            scan.texture_references
        );
    } else {
        inspect_bounded_source(path, extension, &mut scan)?;
        scan.notes = if extension == "usdz" {
            "descriptive package metadata from bounded USD samples; not fixture eligibility"
                .to_owned()
        } else {
            "descriptive bounded source sample; not fixture eligibility".to_owned()
        };
    }
    scan.arcs.sort();
    scan.arcs.dedup();
    Ok(scan)
}

fn inspect_package_entries(path: &Path, scan: &mut AssetScan) -> Result<(), String> {
    let file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("open USDZ {}: {error}", path.display()))?;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("read USDZ entry {}: {error}", path.display()))?;
        let name = entry.name().to_ascii_lowercase();
        if name.ends_with(".png") || name.ends_with(".jpg") || name.ends_with(".jpeg") {
            scan.texture_references += 1;
        }
    }
    Ok(())
}

fn inspect_openusd_stage(path: &Path, scan: &mut AssetScan) -> Result<(), String> {
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(path.to_string_lossy().as_ref())
        .map_err(|error| format!("open canonical USD asset {}: {error}", path.display()))?;
    scan.dependency_count = stage.layer_stack().len().saturating_sub(1);
    scan.arcs.extend(
        stage
            .layer_stack()
            .into_iter()
            .skip(1)
            .map(|identifier| format!("layer:{identifier}")),
    );
    scan.arcs.extend(
        stage
            .composition_errors()
            .iter()
            .map(|error| format!("composition_error:{error}")),
    );

    stage
        .traverse(PrimPredicate::ALL, |path| {
            let prim = stage.prim(path.clone());
            let type_name = prim.type_name().ok().flatten();
            scan.instance_indicator |= type_name.as_deref() == Some("PointInstancer");
            scan.instance_indicator |= prim.is_instanceable().unwrap_or(false);
            scan.animation |= type_name.as_deref() == Some("SkelAnimation");
            if let Ok(attributes) = prim.attributes() {
                scan.animation |= attributes.into_iter().any(|attribute| {
                    attribute
                        .time_sample_times()
                        .is_ok_and(|times| !times.is_empty())
                });
            }
        })
        .map_err(|error| format!("traverse canonical USD asset {}: {error}", path.display()))?;

    let semantic = SemanticExtractor::new(SemanticConfig::for_nvidia_revit_connector())
        .extract(
            &stage,
            SnapshotSource::Working {
                session: "or8-m2-asset-inventory".to_owned(),
                live_revision: 1,
            },
        )
        .map_err(|error| {
            format!(
                "extract canonical asset semantics {}: {error}",
                path.display()
            )
        })?;
    scan.bim_indicator = semantic
        .entities
        .values()
        .any(|entity| entity.semantic.is_bim_entity());
    Ok(())
}

fn inspect_bounded_source(
    path: &Path,
    extension: &str,
    scan: &mut AssetScan,
) -> Result<(), String> {
    if extension == "usdz" {
        let file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|error| format!("open USDZ {}: {error}", path.display()))?;
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|error| format!("read USDZ entry {}: {error}", path.display()))?;
            if is_usd_entry(entry.name()) {
                let mut sample = Vec::new();
                entry
                    .by_ref()
                    .take(SAMPLE_LIMIT)
                    .read_to_end(&mut sample)
                    .map_err(|error| format!("sample USDZ entry {}: {error}", path.display()))?;
                scan_bounded_bytes(&sample, scan);
            }
        }
    } else {
        let mut file =
            File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
        let mut sample = Vec::new();
        file.by_ref()
            .take(SAMPLE_LIMIT)
            .read_to_end(&mut sample)
            .map_err(|error| format!("sample {}: {error}", path.display()))?;
        scan_bounded_bytes(&sample, scan);
    }
    Ok(())
}

fn is_usd_entry(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".usd") || lower.ends_with(".usda") || lower.ends_with(".usdc")
}

fn scan_bounded_bytes(bytes: &[u8], scan: &mut AssetScan) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn filename_does_not_authorize_point_instancer_fixture() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("PointInstancedMedCity.usda");
        fs::write(
            &source,
            "#usda 1.0\n(\n    defaultPrim = \"Root\"\n)\ndef \"Root\" { }\n",
        )
        .unwrap();

        let scan = inspect(&source, "usda", true).unwrap();

        assert!(!scan.instance_indicator);
    }
}
