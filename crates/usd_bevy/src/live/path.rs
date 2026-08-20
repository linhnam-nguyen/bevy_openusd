use anyhow::{Result, anyhow, bail};

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
