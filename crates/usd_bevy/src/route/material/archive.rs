use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use bevy::prelude::*;
use openusd::ar::{CacheScope, DefaultResolver, ResolvedPath, split_package_relative_path_outer};
use openusd::usd::Stage;
use openusd::usdz::{open_package, package_entry};

use super::texture_cache::UsdTextureCache;

#[path = "archive_dependencies.rs"]
mod archive_dependencies;
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

/// Return only the USDZ packages already represented by the Stage and its
/// authored dependency closure. Discovery is intentionally lazy and does not
/// force full composed prim traversal during activation.
pub fn archive_paths_for_stage(stage: &Stage, root_path: &Path) -> anyhow::Result<Vec<PathBuf>> {
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
    for path in archive_dependencies::authored_archive_paths(stage)? {
        add(&path.to_string_lossy());
    }
    for identifier in stage.layer_identifiers() {
        if !identifier.starts_with("anon:") {
            add(&identifier);
        }
    }
    paths.sort();
    Ok(paths)
}

fn build_archive_index(path: &Path, fingerprint: ArchiveFingerprint) -> (ArchiveIndex, u64) {
    let _scope = CacheScope::begin();
    let resolver = DefaultResolver::new();
    let resolved = ResolvedPath::new(path.to_path_buf());
    let mut entries = HashMap::new();
    let Ok(package) = open_package(&resolver, &resolved) else {
        return (
            ArchiveIndex {
                fingerprint,
                entries,
            },
            0,
        );
    };
    let Ok(entry_names) = package.with(|archive| archive.entry_names()) else {
        return (
            ArchiveIndex {
                fingerprint,
                entries,
            },
            0,
        );
    };
    let entries_scanned = entry_names.len() as u64;
    for original_name in entry_names {
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

pub(super) fn read_archive_entry_bytes(package_path: &Path, entry_name: &str) -> Option<Vec<u8>> {
    let _scope = CacheScope::begin();
    let resolver = DefaultResolver::new();
    let package_str = package_path.to_str()?;
    let buffer = package_entry(&resolver, package_str, entry_name).ok()?;
    Some(buffer.to_vec())
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
    let _scope = CacheScope::begin();
    let resolver = DefaultResolver::new();
    for usdz in usdz_files {
        archive_stats.archives_scanned += 1;
        let resolved = ResolvedPath::new(usdz.clone());
        let Ok(package) = open_package(&resolver, &resolved) else {
            continue;
        };
        let Ok(names) = package.with(|archive| archive.entry_names()) else {
            continue;
        };
        for original_name in names {
            archive_stats.entries_scanned += 1;
            let norm_zip = normalized_archive_entry(&original_name);
            if archive_entry_matches(&norm_zip, norm_path)
                && let Some(bytes) = read_archive_entry_bytes(usdz, &original_name)
            {
                archive_stats.hits += 1;
                return Some(bytes);
            }
        }
    }
    None
}
