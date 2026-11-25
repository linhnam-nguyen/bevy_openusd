//! Deterministic policy for visual-only USD composition and authored LOD.
//!
//! This module never reads semantic truth, authors source layers, or owns render
//! buffers. Its output is a request to the one visual-stage composition adapter.
//! The existing `ResidencyAuthority` remains the only CPU/GPU cache authority.
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VisualCandidate {
    pub path: String,
    pub bound_known: bool,
    pub visible: bool,
    pub selected: bool,
    pub camera_mount: bool,
    pub editing: bool,
    /// Projected diameter in physical pixels; `None` disables camera eviction.
    pub projected_pixels: Option<f64>,
    pub distance: f64,
    pub estimated_bytes: u64,
}

impl VisualCandidate {
    fn pinned(&self) -> bool {
        self.selected || self.camera_mount || self.editing
    }

    fn safe_to_stream(&self) -> bool {
        self.bound_known
            && self
                .projected_pixels
                .is_some_and(|v| v.is_finite() && v >= 0.0)
            && self.distance.is_finite()
            && self.distance >= 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VisualPolicyConfig {
    pub enabled: bool,
    pub enter_pixels: f64,
    pub exit_pixels: f64,
    pub max_changes_per_frame: usize,
    pub max_new_bytes_per_frame: u64,
    pub unload_grace_observations: u8,
}

impl Default for VisualPolicyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            enter_pixels: 32.0,
            exit_pixels: 18.0,
            max_changes_per_frame: 8,
            max_new_bytes_per_frame: 64 * 1024 * 1024,
            unload_grace_observations: 3,
        }
    }
}

#[derive(Default, Clone, Debug, PartialEq)]
pub(crate) struct VisualLoadPlan {
    pub load: Vec<String>,
    pub unload: Vec<String>,
    /// Paths held because the metadata cannot prove that unloading is safe.
    pub conservative: Vec<String>,
    pub deferred: usize,
}

#[derive(Default, Clone, Debug)]
pub(crate) struct VisualPolicy {
    cold_observations: BTreeMap<String, u8>,
}

impl VisualPolicy {
    pub fn plan(
        &mut self,
        config: VisualPolicyConfig,
        discovered: &[String],
        loaded: &BTreeSet<String>,
        candidates: &[VisualCandidate],
    ) -> VisualLoadPlan {
        let mut result = VisualLoadPlan::default();
        let known = discovered.iter().cloned().collect::<BTreeSet<_>>();
        // Duplicate candidate paths are merged conservatively. Unknown hints
        // and pins are never weakened by a later occurrence.
        let mut candidate_map = BTreeMap::<String, VisualCandidate>::new();
        for candidate in candidates {
            if !known.contains(&candidate.path) {
                continue;
            }
            candidate_map
                .entry(candidate.path.clone())
                .and_modify(|prior| {
                    prior.selected |= candidate.selected;
                    prior.camera_mount |= candidate.camera_mount;
                    prior.editing |= candidate.editing;
                    prior.visible |= candidate.visible;
                    prior.bound_known &= candidate.bound_known;
                    prior.projected_pixels =
                        match (prior.projected_pixels, candidate.projected_pixels) {
                            (Some(a), Some(b)) => Some(a.max(b)),
                            _ => None,
                        };
                    prior.distance = prior.distance.min(candidate.distance);
                    prior.estimated_bytes = prior.estimated_bytes.max(candidate.estimated_bytes);
                })
                .or_insert_with(|| candidate.clone());
        }

        // For absent/invalid bounds, preserve the currently loaded payload and
        // load an unloaded one: this is a deliberate fail-open-for-visibility
        // policy, never a proof that unknown content is empty.
        let mut desired = BTreeSet::new();
        for path in &known {
            let is_loaded = loaded.contains(path);
            let Some(candidate) = candidate_map.get(path) else {
                desired.insert(path.clone());
                result.conservative.push(path.clone());
                continue;
            };
            let keep = if !config.enabled || candidate.pinned() || !candidate.safe_to_stream() {
                result
                    .conservative
                    .extend((!candidate.safe_to_stream()).then(|| path.clone()));
                true
            } else if !candidate.visible {
                false
            } else {
                let threshold = if is_loaded {
                    config.exit_pixels
                } else {
                    config.enter_pixels
                };
                candidate.projected_pixels.is_some_and(|v| v >= threshold)
            };
            if keep {
                desired.insert(path.clone());
                self.cold_observations.remove(path);
            } else if is_loaded {
                let count = self.cold_observations.entry(path.clone()).or_insert(0);
                *count = count.saturating_add(1);
                if *count <= config.unload_grace_observations {
                    desired.insert(path.clone());
                }
            } else {
                self.cold_observations.remove(path);
            }
        }

        // USD nested payload descendants cannot compose if an unloaded parent
        // excludes them. Keep every known ancestor of a wanted descendant.
        let wanted = desired.iter().cloned().collect::<Vec<_>>();
        for child in wanted {
            for parent in ancestors(&child) {
                if known.contains(parent) {
                    desired.insert(parent.to_owned());
                }
            }
        }
        let mut to_load = desired.difference(loaded).cloned().collect::<Vec<_>>();
        // Parent first; pins first within the same depth; stable tie-break by
        // projected size, then lexical path. Never use HashMap iteration order.
        to_load.sort_by(|a, b| {
            let ac = candidate_map.get(a);
            let bc = candidate_map.get(b);
            path_depth(a)
                .cmp(&path_depth(b))
                .then_with(|| {
                    bc.is_some_and(VisualCandidate::pinned)
                        .cmp(&ac.is_some_and(VisualCandidate::pinned))
                })
                .then_with(|| {
                    let ap = ac.and_then(|c| c.projected_pixels).unwrap_or(f64::INFINITY);
                    let bp = bc.and_then(|c| c.projected_pixels).unwrap_or(f64::INFINITY);
                    bp.total_cmp(&ap)
                })
                .then_with(|| a.cmp(b))
        });
        let mut to_unload = loaded.difference(&desired).cloned().collect::<Vec<_>>();
        // Descendants before ancestors for well-defined batched transitions.
        to_unload.sort_by(|a, b| path_depth(b).cmp(&path_depth(a)).then_with(|| a.cmp(b)));
        let mut transitions = 0;
        let mut new_bytes = 0u64;
        let mut admitted = loaded.clone();
        for path in to_load {
            let parent_deferred =
                ancestors(&path).any(|parent| known.contains(parent) && !admitted.contains(parent));
            if parent_deferred || transitions >= config.max_changes_per_frame {
                result.deferred += 1;
                continue;
            }
            let cost = candidate_map.get(&path).map_or(0, |c| c.estimated_bytes);
            // Permit one oversized mandatory load only when no smaller load
            // has already been issued, to avoid permanent starvation.
            if transitions > 0 && new_bytes.saturating_add(cost) > config.max_new_bytes_per_frame {
                result.deferred += 1;
                continue;
            }
            admitted.insert(path.clone());
            result.load.push(path);
            transitions += 1;
            new_bytes = new_bytes.saturating_add(cost);
        }
        let mut blocked = BTreeSet::new();
        for path in to_unload {
            if blocked.contains(path.as_str()) || transitions >= config.max_changes_per_frame {
                blocked.extend(ancestors(&path).map(str::to_owned));
                result.deferred += 1;
                continue;
            }
            result.unload.push(path);
            transitions += 1;
        }
        result
    }
}

pub(crate) fn is_ancestor_of(parent: &str, child: &str) -> bool {
    child.len() > parent.len()
        && child.starts_with(parent)
        && child.as_bytes().get(parent.len()) == Some(&b'/')
}

fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(path.rsplit_once('/').map(|(parent, _)| parent), |path| {
        path.rsplit_once('/').map(|(parent, _)| parent)
    })
    .filter(|parent| !parent.is_empty())
}

fn path_depth(path: &str) -> usize {
    path.as_bytes().iter().filter(|byte| **byte == b'/').count()
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LodChoice {
    pub set_name: String,
    pub variant_name: String,
    /// Higher = more detailed. Authored geometry is never simplified here.
    pub detail: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LodThresholds {
    pub high_pixels: f64,
    pub medium_pixels: f64,
    pub hysteresis: f64,
}

impl Default for LodThresholds {
    fn default() -> Self {
        Self {
            high_pixels: 240.0,
            medium_pixels: 72.0,
            hysteresis: 0.15,
        }
    }
}

/// Return only a *known authored* choice. Unsupported/missing choices return
/// None so the caller preserves the authored full/default representation.
/// `selected_exact` forbids coarser geometry for selection and measurements.
/// Other variant sets (appearance, configuration, materials) are untouched.
pub(crate) fn choose_authored_lod(
    variants: &[LodChoice],
    pixels: f64,
    previous_detail: Option<usize>,
    selected_exact: bool,
    thresholds: LodThresholds,
) -> Option<LodChoice> {
    if variants.is_empty() || !pixels.is_finite() || pixels < 0.0 {
        return None;
    }
    let mut ordered = variants.to_vec();
    ordered.sort_by(|a, b| {
        b.detail
            .cmp(&a.detail)
            .then_with(|| a.variant_name.cmp(&b.variant_name))
    });
    let top = ordered.first()?.detail;
    let medium = ordered
        .iter()
        .find(|c| c.detail < top)
        .map(|c| c.detail)
        .unwrap_or(top);
    let low = ordered.last()?.detail;
    let h = thresholds.hysteresis.clamp(0.0, 0.45);
    let threshold = |threshold: f64, next_higher: usize| {
        if previous_detail.is_some_and(|prev| prev >= next_higher) {
            threshold * (1.0 - h)
        } else {
            threshold * (1.0 + h)
        }
    };
    let desired = if selected_exact || pixels >= threshold(thresholds.high_pixels, top) {
        top
    } else if pixels >= threshold(thresholds.medium_pixels, medium) {
        medium
    } else {
        low
    };
    ordered.into_iter().find(|c| c.detail == desired)
}

#[cfg(test)]
#[path = "../../../../tests/unit/viewport/residency/visual_policy_tests.rs"]
mod tests;
