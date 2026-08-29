use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use super::catalog::manifest_store::write_bytes_atomic;

pub(crate) const PROJECT_METADATA_DIRECTORY: &str = ".usdhub";
pub(crate) const SCENES_DIRECTORY: &str = "scenes";
pub(crate) const MODELS_DIRECTORY: &str = "models";
pub(crate) const CACHE_DIRECTORY: &str = "cache";
pub(crate) const RECOVERY_DIRECTORY: &str = "recovery";
pub(crate) const LINKS_DIRECTORY: &str = "links";
pub(crate) const CACHE_OBJECTS_RELATIVE_PATH: &str = ".usdhub/cache/objects";
pub(crate) const PROJECT_MANIFEST_FILE: &str = "project.json";

pub(crate) const MANAGED_IGNORE_BEGIN: &str = "# BEGIN USDHub managed local state";
pub(crate) const MANAGED_IGNORE_END: &str = "# END USDHub managed local state";

const MANAGED_IGNORE_BLOCK: &str = concat!(
    "# BEGIN USDHub managed local state\n",
    ".usdhub/cache/\n",
    ".usdhub/recovery/\n",
    ".usdhub/links/\n",
    "# END USDHub managed local state\n",
);

/// Canonical filesystem paths for one local Project.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectStorageLayout {
    root: PathBuf,
}

impl ProjectStorageLayout {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn metadata_dir(&self) -> PathBuf {
        self.root.join(PROJECT_METADATA_DIRECTORY)
    }

    pub(crate) fn manifest_path(&self) -> PathBuf {
        self.metadata_dir().join(PROJECT_MANIFEST_FILE)
    }

    pub(crate) fn scenes_dir(&self) -> PathBuf {
        self.metadata_dir().join(SCENES_DIRECTORY)
    }

    pub(crate) fn models_dir(&self) -> PathBuf {
        self.metadata_dir().join(MODELS_DIRECTORY)
    }

    pub(crate) fn cache_dir(&self) -> PathBuf {
        self.metadata_dir().join(CACHE_DIRECTORY)
    }

    pub(crate) fn cache_objects_dir(&self) -> PathBuf {
        self.cache_dir().join("objects")
    }

    pub(crate) fn recovery_dir(&self) -> PathBuf {
        self.metadata_dir().join(RECOVERY_DIRECTORY)
    }

    pub(crate) fn links_dir(&self) -> PathBuf {
        self.metadata_dir().join(LINKS_DIRECTORY)
    }

    /// Bootstrap only the local-state roots; object/session children stay lazy.
    pub(crate) fn ensure_local_state_roots(&self) -> Result<()> {
        fs::create_dir_all(self.cache_dir()).context("create Project cache root")?;
        fs::create_dir_all(self.recovery_dir()).context("create Project recovery root")?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IgnoreChange {
    pub(crate) original: Option<Vec<u8>>,
    pub(crate) changed: bool,
}

pub(crate) fn read_gitignore(root: &Path) -> Result<Option<Vec<u8>>> {
    let path = root.join(".gitignore");
    match fs::read(&path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

pub(crate) fn has_broad_usdhub_ignore(bytes: &[u8]) -> Result<bool> {
    let text = std::str::from_utf8(bytes).context("Project .gitignore must be UTF-8")?;
    Ok(text.lines().any(is_broad_usdhub_rule))
}

pub(crate) fn install_managed_ignore(root: &Path) -> Result<IgnoreChange> {
    let path = root.join(".gitignore");
    let original = read_gitignore(root)?;
    let existing = original.as_deref().unwrap_or_default();
    if has_broad_usdhub_ignore(existing)? {
        bail!("IgnoreConflict: a broad .usdhub rule hides canonical Project metadata");
    }

    let merged = merge_managed_ignore(existing)?;
    if merged == existing {
        return Ok(IgnoreChange {
            original,
            changed: false,
        });
    }
    let temporary_path = root.join(format!(".gitignore.{}.tmp", uuid::Uuid::new_v4()));
    write_bytes_atomic(&temporary_path, &path, &merged)
        .with_context(|| format!("publish managed {}", path.display()))?;
    Ok(IgnoreChange {
        original,
        changed: true,
    })
}

pub(crate) fn restore_gitignore(root: &Path, change: &IgnoreChange) -> Result<()> {
    if !change.changed {
        return Ok(());
    }
    let path = root.join(".gitignore");
    match &change.original {
        Some(bytes) => {
            let temporary_path =
                root.join(format!(".gitignore.restore.{}.tmp", uuid::Uuid::new_v4()));
            write_bytes_atomic(&temporary_path, &path, bytes)?;
        }
        None => match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("remove {}", path.display())),
        },
    }
    Ok(())
}

pub(crate) fn merge_managed_ignore(existing: &[u8]) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(existing).context("Project .gitignore must be UTF-8")?;
    let lines = line_ranges(text);
    let begin = lines
        .iter()
        .position(|(_, _, line)| normalized_line(line) == MANAGED_IGNORE_BEGIN);
    if let Some(begin_index) = begin {
        let end_index = lines
            .iter()
            .enumerate()
            .skip(begin_index + 1)
            .find_map(|(index, (_, _, line))| {
                (normalized_line(line) == MANAGED_IGNORE_END).then_some(index)
            })
            .context("managed .gitignore block has no end marker")?;
        if lines
            .iter()
            .skip(end_index + 1)
            .any(|(_, _, line)| normalized_line(line) == MANAGED_IGNORE_BEGIN)
        {
            bail!("managed .gitignore block is duplicated");
        }
        let start = lines[begin_index].0;
        let end = lines[end_index].1;
        let mut merged = String::with_capacity(text.len() + MANAGED_IGNORE_BLOCK.len());
        merged.push_str(&text[..start]);
        merged.push_str(MANAGED_IGNORE_BLOCK);
        merged.push_str(&text[end..]);
        return Ok(merged.into_bytes());
    }

    let mut merged = existing.to_vec();
    if !merged.is_empty() && !merged.ends_with(b"\n") {
        merged.push(b'\n');
    }
    merged.extend_from_slice(MANAGED_IGNORE_BLOCK.as_bytes());
    Ok(merged)
}

fn line_ranges(text: &str) -> Vec<(usize, usize, &str)> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (index, character) in text.char_indices() {
        if character == '\n' {
            ranges.push((start, index + 1, &text[start..index + 1]));
            start = index + 1;
        }
    }
    if start < text.len() {
        ranges.push((start, text.len(), &text[start..]));
    }
    ranges
}

fn normalized_line(line: &str) -> &str {
    line.trim_end_matches('\n').trim_end_matches('\r')
}

fn is_broad_usdhub_rule(line: &str) -> bool {
    let rule = line.trim();
    if rule.is_empty() || rule.starts_with('#') || rule.starts_with('!') {
        return false;
    }
    let rule = rule.trim_end_matches('/');
    let rule = rule.strip_prefix("**/").unwrap_or(rule);
    let rule = rule.strip_prefix('/').unwrap_or(rule);
    rule == ".usdhub" || matches!(rule.strip_prefix(".usdhub/"), Some("*" | "**"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn layout_matches_existing_project_storage_contract() {
        let layout = ProjectStorageLayout::new("project");
        assert_eq!(
            layout.manifest_path(),
            PathBuf::from("project/.usdhub/project.json")
        );
        assert_eq!(layout.scenes_dir(), PathBuf::from("project/.usdhub/scenes"));
        assert_eq!(layout.models_dir(), PathBuf::from("project/.usdhub/models"));
        assert_eq!(
            layout.cache_objects_dir(),
            PathBuf::from("project/.usdhub/cache/objects")
        );
        assert_eq!(
            layout.recovery_dir(),
            PathBuf::from("project/.usdhub/recovery")
        );
    }

    #[test]
    fn managed_ignore_is_idempotent_and_preserves_unrelated_bytes() {
        let existing = b"target/\n# local note\n";
        let first = merge_managed_ignore(existing).unwrap();
        let second = merge_managed_ignore(&first).unwrap();

        assert_eq!(first, second);
        assert!(first.starts_with(existing));
        assert_eq!(first.iter().filter(|byte| **byte == b'\n').count(), 6);
    }

    #[test]
    fn managed_ignore_replaces_only_its_own_block() {
        let existing = b"keep-before\n# BEGIN USDHub managed local state\nold\n# END USDHub managed local state\nkeep-after\n";
        let merged = merge_managed_ignore(existing).unwrap();
        assert_eq!(
            String::from_utf8(merged).unwrap(),
            "keep-before\n# BEGIN USDHub managed local state\n.usdhub/cache/\n.usdhub/recovery/\n# END USDHub managed local state\nkeep-after\n"
        );
    }

    #[test]
    fn broad_usdhub_rule_is_a_conflict() {
        for rule in [
            ".usdhub/",
            "/.usdhub/",
            "**/.usdhub",
            ".usdhub/*",
            ".usdhub/**",
            "/.usdhub/*",
            "**/.usdhub/**",
        ] {
            assert!(has_broad_usdhub_ignore(rule.as_bytes()).unwrap(), "{rule}");
        }
        assert!(!has_broad_usdhub_ignore(b".usdhub/cache/\n").unwrap());
        assert!(!has_broad_usdhub_ignore(b".usdhub/recovery/**\n").unwrap());
    }

    #[test]
    fn local_state_roots_are_bootstrapped_without_object_or_session_children() {
        let directory = tempdir().unwrap();
        let layout = ProjectStorageLayout::new(directory.path());
        layout.ensure_local_state_roots().unwrap();

        assert!(layout.cache_dir().is_dir());
        assert!(layout.recovery_dir().is_dir());
        assert!(!layout.cache_objects_dir().exists());
        assert_eq!(fs::read_dir(layout.recovery_dir()).unwrap().count(), 0);
    }
}
