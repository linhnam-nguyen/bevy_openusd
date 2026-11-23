//! Camera focus uses canonical USD geometry, never the currently resident
//! visual-Stage subtree. A native camera unload must not shrink FocusTarget.
//! This helper is read-only: no source layer, cache, or load rule is authored.

use std::collections::HashSet;

use bevy::prelude::Vec3;
use openusd_schemas::geom::{BBoxCache, Purpose};

/// Source-complete world bounds for a uniquely mapped canonical USD prim.
/// BBoxCache deliberately ignores authored extentsHint, since such hints do
/// not prove that an unloaded nested payload belongs inside the fitted region.
/// The Bevy Stage root applies the up-axis correction once.
fn canonical_fit_params(stage: &usd_bevy::LiveStage, path: &str) -> Option<(Vec3, f32)> {
    let prim = stage.stage.prim(path).ok()?;
    if !prim.is_defined().ok()? {
        return None;
    }
    let range = BBoxCache::new(None, Purpose::ORDERED)
        .with_extents_hint(false)
        .with_ignore_visibility(true)
        .world_bound(&prim)
        .ok()?
        .aligned_range();
    if range.is_empty() {
        return None;
    }
    let source_min: [f64; 3] = range.min().into();
    let source_max: [f64; 3] = range.max().into();
    if !source_min.into_iter().chain(source_max).all(f64::is_finite) {
        return None;
    }

    let up_axis = stage.stage_orientation();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for corner in 0..8 {
        let point = Vec3::new(
            (if corner & 1 == 0 {
                source_min[0]
            } else {
                source_max[0]
            }) as f32,
            (if corner & 2 == 0 {
                source_min[1]
            } else {
                source_max[1]
            }) as f32,
            (if corner & 4 == 0 {
                source_min[2]
            } else {
                source_max[2]
            }) as f32,
        );
        if !point.is_finite() {
            return None;
        }
        let world = up_axis * point;
        if !world.is_finite() {
            return None;
        }
        min = min.min(world);
        max = max.max(world);
    }

    let center = (min + max) * 0.5;
    let size = (max - min).abs();
    let distance = size.x.max(size.y).max(size.z).max(0.05) * 1.6;
    (center.is_finite() && distance.is_finite() && distance > 0.0).then_some((center, distance))
}

/// Keep the same canonical fit while a payload's projection catches up with
/// its native load set. Unaffected anchors and loaded occurrences retain their
/// existing entity fit. An unloaded occurrence has no validated canonical pose.
pub(super) fn focus_params_for_payload(
    stage: &usd_bevy::LiveStage,
    path: &str,
    unique_prim: bool,
) -> Result<Option<(Vec3, f32)>, &'static str> {
    // Only a documentless Stage may keep the existing entity-only fit without
    // querying native visual ownership. A file-backed Stage whose discovery
    // fails cannot prove the current Bevy subtree is a complete focus target.
    let Some(owners) = stage
        .visual_loadable_paths_if_backed()
        .map_err(|_| "cannot verify visual payload owners for focus")?
    else {
        return Ok(None);
    };
    let mut overlapping = owners
        .into_iter()
        .filter(|owner| {
            path == "/"
                || owner == path
                || owner
                    .strip_prefix(path)
                    .is_some_and(|suffix| suffix.starts_with('/'))
                || path
                    .strip_prefix(owner.as_str())
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
        .peekable();
    if overlapping.peek().is_none() {
        return Ok(None);
    }
    if let Some(fit) = unique_prim
        .then(|| canonical_fit_params(stage, path))
        .flatten()
    {
        return Ok(Some(fit));
    }
    let loaded: HashSet<_> = stage
        .visual_loaded_paths()
        .map_err(|_| "cannot verify payload residency for focus")?
        .into_iter()
        .collect();
    if overlapping.any(|owner| !loaded.contains(&owner)) {
        return Err("cannot fit an unloaded payload without complete source bounds");
    }
    Ok(None)
}
