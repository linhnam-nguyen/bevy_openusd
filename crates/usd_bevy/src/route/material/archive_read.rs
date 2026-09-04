use std::io::Read;
use std::path::{Path, PathBuf};

use bevy::prelude::World;
use openusd::ar::split_package_relative_path_outer;

use super::{
    ArchiveLookupStats, archive_entry_matches, canonical_archive_path, collect_usdz_files,
    ensure_archive_index, normalized_archive_entry, scan_archives_without_index,
};
use crate::route::material::texture_cache::UsdTextureCache;

pub(crate) fn read_texture_bytes(
    world: &mut World,
    texture_path: &str,
) -> (Option<Vec<u8>>, ArchiveLookupStats) {
    let mut archive_stats = ArchiveLookupStats::default();
    if openusd::ar::is_package_relative_path(texture_path) {
        let Some((package, inner)) = split_package_relative_path_outer(texture_path) else {
            return (None, archive_stats);
        };
        let package_path = Path::new(&package);
        let Some(package_path) = package_path
            .exists()
            .then(|| canonical_archive_path(package_path))
        else {
            return (None, archive_stats);
        };
        let stats = ensure_archive_index(world, &package_path);
        archive_stats.archives_scanned += stats.archives_scanned;
        archive_stats.entries_scanned += stats.entries_scanned;
        archive_stats.index_builds += stats.index_builds;
        archive_stats.index_invalidations += stats.index_invalidations;
        archive_stats.entries_indexed += stats.entries_indexed;
        let entry_name = world
            .get_resource::<UsdTextureCache>()
            .and_then(|cache| cache.archive_indices().get(&package_path))
            .and_then(|index| {
                index
                    .entries
                    .iter()
                    .find(|(entry, _)| {
                        archive_entry_matches(entry, &normalized_archive_entry(&inner))
                    })
                    .map(|(_, original)| original.clone())
            });
        let Some(entry_name) = entry_name else {
            archive_stats.misses = 1;
            return (None, archive_stats);
        };
        let Ok(file) = std::fs::File::open(&package_path) else {
            return (None, archive_stats);
        };
        let Ok(mut archive) = zip::ZipArchive::new(file) else {
            return (None, archive_stats);
        };
        let Ok(mut zip_file) = archive.by_name(&entry_name) else {
            return (None, archive_stats);
        };
        let mut buffer = Vec::new();
        if zip_file.read_to_end(&mut buffer).is_ok() {
            archive_stats.hits = 1;
            return (Some(buffer), archive_stats);
        }
        return (None, archive_stats);
    }
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
    let usdz_files = collect_usdz_files(world);
    if usdz_files.len() > 1 {
        archive_stats.misses = 1;
        return (None, archive_stats);
    }
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
