use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use bevy::prelude::*;
use openusd::ar::split_package_relative_path_outer;
use openusd::usd::{PrimPredicate, Stage};

use super::texture_cache::UsdTextureCache;

#[path = "archive_read.rs"]
mod archive_read;
pub(super) use archive_read::read_texture_bytes;

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

/// Return only the USDZ packages reached by the active composed Stage.
/// Traversal forces on-demand reference/payload layers to load; no repository
/// directory is searched and no per-frame discovery is performed.
pub fn archive_paths_for_stage(stage: &Stage, root_path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    stage.traverse(PrimPredicate::DEFAULT, |_| {})?;
    let mut paths = Vec::new();
    let mut add = |identifier: &str| {
        let outer = split_package_relative_path_outer(identifier)
            .map_or_else(|| identifier.to_owned(), |(outer, _)| outer);
        let path = Path::new(&outer);
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("usdz"))
        {
            push_unique_usdz(&mut paths, path.to_path_buf());
        }
    };
    add(&root_path.to_string_lossy());
    for identifier in stage.layer_identifiers() {
        if !identifier.starts_with("anon:") {
            add(&identifier);
        }
    }
    paths.sort();
    Ok(paths)
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
