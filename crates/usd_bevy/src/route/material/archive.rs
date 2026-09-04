use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use bevy::prelude::*;

use super::texture_cache::UsdTextureCache;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ArchiveLookupStats {
    pub(super) archives_scanned: u64,
    pub(super) entries_scanned: u64,
    pub(super) hits: u64,
    pub(super) misses: u64,
    pub(super) index_builds: u64,
    pub(super) index_invalidations: u64,
    pub(super) entries_indexed: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ArchiveFingerprint {
    length: u64,
    modified_ns: Option<u128>,
}

impl ArchiveFingerprint {
    fn read(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos());
        Some(Self {
            length: metadata.len(),
            modified_ns,
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct ArchiveIndex {
    fingerprint: ArchiveFingerprint,
    pub(super) entries: HashMap<String, String>,
}

fn normalized_archive_entry(name: &str) -> String {
    name.replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_owned()
}

fn archive_entry_matches(entry: &str, requested: &str) -> bool {
    entry == requested
}

fn canonical_archive_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn push_unique_usdz(files: &mut Vec<PathBuf>, path: PathBuf) {
    let path = canonical_archive_path(&path);
    if !files.contains(&path) {
        files.push(path);
    }
}

fn collect_usdz_files(world: &World) -> Vec<PathBuf> {
    let mut usdz_files = Vec::new();
    if let Some(cache) = world.get_resource::<UsdTextureCache>() {
        for path in &cache.archive_paths {
            if path
                .extension()
                .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("usdz"))
            {
                push_unique_usdz(&mut usdz_files, path.clone());
            }
        }
    }
    usdz_files
}

fn build_archive_index(path: &Path, fingerprint: ArchiveFingerprint) -> (ArchiveIndex, u64) {
    let mut entries = HashMap::new();
    let Ok(file) = std::fs::File::open(path) else {
        return (
            ArchiveIndex {
                fingerprint,
                entries,
            },
            0,
        );
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return (
            ArchiveIndex {
                fingerprint,
                entries,
            },
            0,
        );
    };

    let entries_scanned = archive.len() as u64;
    for index in 0..archive.len() {
        let Ok(zip_file) = archive.by_index(index) else {
            continue;
        };
        let original_name = zip_file.name().to_owned();
        entries
            .entry(normalized_archive_entry(&original_name))
            .or_insert(original_name);
    }

    (
        ArchiveIndex {
            fingerprint,
            entries,
        },
        entries_scanned,
    )
}

fn ensure_archive_index(world: &mut World, path: &Path) -> ArchiveLookupStats {
    let Some(fingerprint) = ArchiveFingerprint::read(path) else {
        return ArchiveLookupStats::default();
    };
    let path = canonical_archive_path(path);
    let (needs_build, invalidated) = world
        .get_resource::<UsdTextureCache>()
        .map(|cache| match cache.archive_indices().get(&path) {
            Some(index) if index.fingerprint == fingerprint => (false, false),
            Some(_) => (true, true),
            None => (true, false),
        })
        .unwrap_or((false, false));
    if !needs_build {
        return ArchiveLookupStats::default();
    }

    let (index, entries_scanned) = build_archive_index(&path, fingerprint);
    let entries_indexed = index.entries.len() as u64;
    if let Some(mut cache) = world.get_resource_mut::<UsdTextureCache>() {
        cache.insert_archive_index(path, index);
    }
    ArchiveLookupStats {
        archives_scanned: 1,
        entries_scanned,
        index_builds: 1,
        index_invalidations: u64::from(invalidated),
        entries_indexed,
        ..Default::default()
    }
}

fn scan_archives_without_index(
    usdz_files: &[PathBuf],
    norm_path: &str,
    archive_stats: &mut ArchiveLookupStats,
) -> Option<Vec<u8>> {
    for usdz in usdz_files {
        archive_stats.archives_scanned += 1;
        let Ok(file) = std::fs::File::open(usdz) else {
            continue;
        };
        let Ok(mut archive) = zip::ZipArchive::new(file) else {
            continue;
        };

        for index in 0..archive.len() {
            archive_stats.entries_scanned += 1;
            let Ok(mut zip_file) = archive.by_index(index) else {
                continue;
            };
            let norm_zip = normalized_archive_entry(zip_file.name());
            if archive_entry_matches(&norm_zip, norm_path) {
                let mut buffer = Vec::new();
                if zip_file.read_to_end(&mut buffer).is_ok() {
                    archive_stats.hits += 1;
                    return Some(buffer);
                }
            }
        }
    }
    None
}

pub(super) fn read_texture_bytes(
    world: &mut World,
    texture_path: &str,
) -> (Option<Vec<u8>>, ArchiveLookupStats) {
    let mut archive_stats = ArchiveLookupStats::default();
    let raw_path = Path::new(texture_path);
    if raw_path.is_absolute()
        && raw_path.exists()
        && let Ok(bytes) = std::fs::read(raw_path)
    {
        return (Some(bytes), archive_stats);
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = [
        manifest_dir.join(texture_path),
        manifest_dir.join("assets").join(texture_path),
        manifest_dir.join("assets/external").join(texture_path),
        manifest_dir.join("../..").join(texture_path),
        PathBuf::from(texture_path),
        PathBuf::from("assets").join(texture_path),
        PathBuf::from("assets/external").join(texture_path),
    ];
    for candidate in &candidates {
        if candidate.exists()
            && let Ok(bytes) = std::fs::read(candidate)
        {
            return (Some(bytes), archive_stats);
        }
    }

    let norm_path = normalized_archive_entry(texture_path);
    // A stage/package is registered by the lifecycle when it becomes active.
    // Do not discover unrelated repository archives here: the material path
    // must remain bounded by the packages that own the active stage.
    let usdz_files = collect_usdz_files(world);
    if world.get_resource::<UsdTextureCache>().is_some() {
        for usdz in &usdz_files {
            let stats = ensure_archive_index(world, usdz);
            archive_stats.archives_scanned += stats.archives_scanned;
            archive_stats.entries_scanned += stats.entries_scanned;
            archive_stats.index_builds += stats.index_builds;
            archive_stats.index_invalidations += stats.index_invalidations;
            archive_stats.entries_indexed += stats.entries_indexed;
            let path = canonical_archive_path(usdz);
            let entry_name = world
                .get_resource::<UsdTextureCache>()
                .and_then(|cache| cache.archive_indices().get(&path))
                .and_then(|index| {
                    index
                        .entries
                        .iter()
                        .find(|(entry, _)| archive_entry_matches(entry, &norm_path))
                        .map(|(_, original)| original.clone())
                });
            let Some(entry_name) = entry_name else {
                continue;
            };
            let Ok(file) = std::fs::File::open(usdz) else {
                continue;
            };
            let Ok(mut archive) = zip::ZipArchive::new(file) else {
                continue;
            };
            let Ok(mut zip_file) = archive.by_name(&entry_name) else {
                continue;
            };
            let mut buffer = Vec::new();
            if zip_file.read_to_end(&mut buffer).is_ok() {
                archive_stats.hits += 1;
                return (Some(buffer), archive_stats);
            }
        }
    } else if let Some(bytes) =
        scan_archives_without_index(&usdz_files, &norm_path, &mut archive_stats)
    {
        return (Some(bytes), archive_stats);
    }

    if !usdz_files.is_empty() {
        archive_stats.misses = 1;
    }
    (None, archive_stats)
}
