use anyhow::{Result, anyhow, bail};
use bevy::prelude::Resource;
use std::collections::HashMap;

/// Compact identity for one canonical prim path in [`PathStore`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PathId(u32);

/// The sole owner of canonical prim-path bytes used by projection indexes.
///
/// The hash table stores only compact IDs; path strings live once in `paths`.
/// Hash collisions are resolved by comparing the canonical strings in the
/// bucket, so the ID remains lossless without duplicating string keys.
#[derive(Resource, Debug, Default)]
pub struct PathStore {
    paths: Vec<String>,
    by_hash: HashMap<u64, Vec<PathId>>,
    namespace_children: HashMap<PathId, Vec<PathId>>,
}

impl PathStore {
    /// Intern a prim path and return its stable ID for the current store.
    pub fn intern(&mut self, path: impl AsRef<str>) -> PathId {
        let normalized = normalize_prim_path(path.as_ref());
        if let Some(id) = self.lookup_exact(&normalized) {
            return id;
        }

        if normalized == "/" {
            return self.insert_owned(normalized);
        }

        if self.lookup_exact("/").is_none() {
            self.insert_owned("/".to_string());
        }

        // Keep every namespace ancestor available for topology traversal and
        // ancestor dependency queries. Each canonical path is still owned by
        // exactly one Vec entry, rather than once per index edge.
        let mut current = String::with_capacity(normalized.len());
        for segment in normalized.split('/').filter(|segment| !segment.is_empty()) {
            current.push('/');
            current.push_str(segment);
            if self.lookup_exact(&current).is_none() {
                self.insert_owned(current.clone());
            }
        }
        self.lookup_exact(&normalized)
            .expect("interned canonical path is present")
    }

    /// Find an already interned path without retaining a new string.
    pub fn lookup(&self, path: &str) -> Option<PathId> {
        self.lookup_exact(path).or_else(|| {
            let normalized = normalize_prim_path(path);
            self.lookup_exact(&normalized)
        })
    }

    /// Resolve one compact path ID to its canonical path bytes.
    pub fn path(&self, id: PathId) -> Option<&str> {
        self.paths
            .get(usize::try_from(id.0).ok()?)
            .map(String::as_str)
    }

    /// Return the interned namespace parent, if any.
    pub fn parent(&self, id: PathId) -> Option<PathId> {
        let path = self.path(id)?;
        match path.rfind('/') {
            Some(0) => self.lookup_exact("/"),
            Some(index) => self.lookup_exact(&path[..index]),
            None => None,
        }
    }

    /// Visit interned ancestors from the path itself toward the stage root.
    /// The callback receives borrowed IDs and no prefix strings are created.
    pub fn for_each_ancestor_id(&self, path: &str, mut visit: impl FnMut(PathId)) {
        let mut end = path.len();
        loop {
            if let Some(id) = self.lookup_exact(&path[..end]) {
                visit(id);
            }
            if end <= 1 {
                break;
            }
            end = path[..end].rfind('/').unwrap_or(0).max(1);
        }
    }

    /// Visit an interned path and every namespace descendant using compact IDs.
    ///
    /// The topology is owned once by the shared path store, so dependency indexes
    /// can answer descendant queries without retaining prefix postings or scanning
    /// their complete reverse maps.
    pub fn for_each_descendant_id(&self, root: PathId, mut visit: impl FnMut(PathId)) {
        let mut pending = vec![root];
        while let Some(path) = pending.pop() {
            visit(path);
            if let Some(children) = self.namespace_children.get(&path) {
                pending.extend(children.iter().copied());
            }
        }
    }

    /// Compare two interned paths using namespace boundaries.
    pub fn is_descendant_or_self(&self, ancestor: PathId, candidate: PathId) -> bool {
        let Some(ancestor) = self.path(ancestor) else {
            return false;
        };
        let Some(candidate) = self.path(candidate) else {
            return false;
        };
        if ancestor == "/" || ancestor == candidate {
            return true;
        }
        candidate
            .strip_prefix(ancestor)
            .is_some_and(|suffix| suffix.starts_with('/') || suffix.starts_with('.'))
    }

    /// Number of unique canonical paths owned by the store.
    pub fn len(&self) -> usize {
        self.paths.len()
    }

    /// Number of bytes retained for canonical path strings.
    pub fn path_bytes(&self) -> usize {
        self.paths.iter().map(String::len).sum()
    }

    /// Whether no canonical paths are retained.
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Reset the store after all projection indexes have been cleared.
    pub(crate) fn clear(&mut self) {
        self.paths.clear();
        self.by_hash.clear();
        self.namespace_children.clear();
    }

    fn lookup_exact(&self, path: &str) -> Option<PathId> {
        let hash = path_hash(path);
        self.by_hash
            .get(&hash)?
            .iter()
            .copied()
            .find(|id| self.path(*id).is_some_and(|candidate| candidate == path))
    }

    fn insert_owned(&mut self, path: String) -> PathId {
        let parent = match path.rfind('/') {
            Some(0) => self.lookup_exact("/"),
            Some(index) => self.lookup_exact(&path[..index]),
            None => None,
        };
        let id = PathId(
            self.paths
                .len()
                .try_into()
                .expect("path store exceeds u32 IDs"),
        );
        let hash = path_hash(&path);
        self.paths.push(path);
        self.by_hash.entry(hash).or_default().push(id);
        if let Some(parent) = parent {
            self.namespace_children.entry(parent).or_default().push(id);
        }
        id
    }
}

fn path_hash(path: &str) -> u64 {
    // Stable FNV-1a keeps the interner independent of randomized hash seeds.
    path.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

/// Normalizes a USD prim or property path to its owning prim path without trailing slashes.
///
/// Leading `/` is ensured, property specifiers (`.property_name`) are stripped defensively,
/// and trailing slashes are removed unless the path is the root `"/"`.
pub fn normalize_prim_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return "/".to_string();
    }
    let without_prop = match trimmed.split_once('.') {
        Some((prim, _prop)) => prim,
        None => trimmed,
    };
    let mut normalized = without_prop.to_string();
    if !normalized.starts_with('/') {
        normalized.insert(0, '/');
    }
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    if normalized.is_empty() {
        "/".to_string()
    } else {
        normalized
    }
}

/// Validates that a path string can safely represent a normalized OpenUSD prim path.
///
/// Returns the normalized path if valid, or an error if the path contains invalid syntax,
/// unresolvable relative components, or cannot be parsed by OpenUSD.
pub fn validate_prim_path(path: &str) -> Result<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return Ok("/".to_string());
    }
    if trimmed.contains("//") || trimmed.contains("..") || trimmed.split('/').any(|seg| seg == ".")
    {
        bail!("path '{path}' contains unsafe relative or empty segments");
    }
    let normalized = normalize_prim_path(path);
    if normalized == "/" {
        return Ok(normalized);
    }
    openusd::sdf::path(&normalized)
        .map_err(|e| anyhow!("invalid OpenUSD prim path '{normalized}': {e:#}"))?;
    Ok(normalized)
}

/// Checks whether `candidate` is equal to or a descendant of `ancestor` with boundary awareness.
///
/// This prevents naive substring matches like `/World/A` falsely matching `/World/AB`.
pub fn is_descendant_or_self(ancestor: &str, candidate: &str) -> bool {
    let ancestor = normalize_prim_path(ancestor);
    let candidate = normalize_prim_path(candidate);

    if ancestor == "/" {
        return true;
    }
    if ancestor == candidate {
        return true;
    }
    if candidate.starts_with(&ancestor) {
        let after_ancestor = &candidate[ancestor.len()..];
        return after_ancestor.starts_with('/') || after_ancestor.starts_with('.');
    }
    false
}

/// Normalizes and minimizes a set of resync candidate paths.
///
/// Deduplicates exact duplicates, sorts shallowest first, and prunes any child path
/// whose ancestor is already included. If the stage root `"/"` is present, returns `["/"]`.
pub fn minimize_resync_roots<I, S>(paths: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut normalized_set = std::collections::HashSet::new();
    for p in paths {
        let norm = normalize_prim_path(p.as_ref());
        if norm == "/" {
            return vec!["/".to_string()];
        }
        normalized_set.insert(norm);
    }

    if normalized_set.is_empty() {
        return Vec::new();
    }

    let mut sorted: Vec<String> = normalized_set.into_iter().collect();
    // Sort primarily by segment depth (fewer '/' means shallower root), secondarily lexicographically
    sorted.sort_by(|a, b| {
        let depth_a = a.matches('/').count();
        let depth_b = b.matches('/').count();
        depth_a.cmp(&depth_b).then_with(|| a.cmp(b))
    });

    let mut accepted: Vec<String> = Vec::new();
    for candidate in sorted {
        let is_covered = accepted
            .iter()
            .any(|root| is_descendant_or_self(root, &candidate));
        if !is_covered {
            accepted.push(candidate);
        }
    }
    accepted
}

/// The namespace parent of a prim path — the pseudo-root `/` for a top-level
/// prim, so it parents onto the stage-root entity.
pub(super) fn parent_path(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &path[..i],
    }
}

/// The prim path owning a (possibly property) path: `/Foo.bar` → `/Foo`.
pub fn prim_of(path: &str) -> &str {
    path.split('.').next().unwrap_or(path)
}

/// The property part of a (possibly property) path: `/Foo.xformOp:x` →
/// `Some("xformOp:x")`; a bare prim path → `None`.
pub fn property_of(path: &str) -> Option<&str> {
    path.split_once('.').map(|(_, prop)| prop)
}
