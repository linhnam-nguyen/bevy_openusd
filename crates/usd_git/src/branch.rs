use std::{collections::{BTreeMap, BTreeSet, HashSet}, fs, path::{Path, PathBuf}, sync::atomic::AtomicBool};

use gix::bstr::ByteSlice;
use gix::refs::transaction::{Change, PreviousValue, RefEdit};
use gix::refs::{FullName, Target};

use crate::{Error, Result, RevisionId};

use super::Repository;

/// A validated local branch name. Remote and repository-ref syntax never
/// crosses this boundary.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BranchName(String);

impl BranchName {
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        let full_name = format!("refs/heads/{name}");
        let valid = !name.is_empty()
            && !name.starts_with('-')
            && FullName::try_from(full_name.as_str()).is_ok();
        if !valid {
            return Err(Error::InvalidBranchName(name));
        }
        Ok(Self(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkingTreeStatus {
    pub dirty: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BranchSwitchOutcome {
    Unchanged { branch: String },
    Switched { from: Option<String>, to: String },
}

pub(super) fn working_tree_status(repository: &Repository) -> Result<WorkingTreeStatus> {
    let dirty = repository.inner.is_dirty().map_err(Error::git)?
        || repository
            .inner
            .status(gix::progress::Discard)
            .map_err(Error::git)?
            .untracked_files(gix::status::UntrackedFiles::Files)
            .into_index_worktree_iter(Vec::new())
            .map_err(Error::git)?
            .next()
            .is_some();
    Ok(WorkingTreeStatus { dirty })
}



pub(super) fn changed_paths_between(
    repository: &Repository,
    from: &RevisionId,
    to: &RevisionId,
) -> Result<Vec<PathBuf>> {
    if from == to {
        return Ok(Vec::new());
    }
    let mut before = BTreeMap::new();
    let mut after = BTreeMap::new();
    collect_tree_ids(&repository.commit_for_id(from)?.tree().map_err(Error::git)?, Path::new(""), &mut before)?;
    collect_tree_ids(&repository.commit_for_id(to)?.tree().map_err(Error::git)?, Path::new(""), &mut after)?;
    let mut paths = BTreeSet::new();
    for path in before.keys().chain(after.keys()) {
        if before.get(path) != after.get(path) {
            paths.insert(path.clone());
        }
    }
    Ok(paths.into_iter().collect())
}

fn collect_tree_ids(
    tree: &gix::Tree<'_>,
    relative: &Path,
    out: &mut BTreeMap<PathBuf, gix::ObjectId>,
) -> Result<()> {
    for entry in tree.iter() {
        let entry = entry.map_err(Error::git)?;
        let filename = std::str::from_utf8(entry.filename().as_ref())
            .map_err(|_| Error::InvalidPath(relative.join("<non-utf8>")))?;
        let path = relative.join(filename);
        match entry.kind() {
            gix::object::tree::EntryKind::Tree => {
                collect_tree_ids(&entry.object().map_err(Error::git)?.into_tree(), &path, out)?;
            }
            gix::object::tree::EntryKind::Blob
            | gix::object::tree::EntryKind::BlobExecutable
            | gix::object::tree::EntryKind::Link => {
                out.insert(path, entry.object_id());
            }
            gix::object::tree::EntryKind::Commit => {
                out.insert(path, entry.object_id());
            }
        }
    }
    Ok(())
}

pub(super) fn switch_branch(
    repository: &mut Repository,
    name: &BranchName,
) -> Result<BranchSwitchOutcome> {
    let from = repository
        .inner
        .head_name()
        .map_err(Error::git)?
        .map(|name| String::from_utf8_lossy(name.shorten()).into_owned());
    if from.as_deref() == Some(name.as_str()) {
        return Ok(BranchSwitchOutcome::Unchanged {
            branch: name.as_str().to_owned(),
        });
    }
    if working_tree_status(repository)?.dirty {
        return Err(Error::DirtyWorkingTree);
    }

    let target_name = format!("refs/heads/{}", name.as_str());
    let target_full_name = FullName::try_from(target_name.as_str())
        .map_err(|_| Error::InvalidBranchName(name.as_str().to_owned()))?;
    let mut target = repository
        .inner
        .find_reference(&target_full_name)
        .map_err(|_| Error::BranchNotFound(name.as_str().to_owned()))?;
    let target_commit = target.peel_to_commit().map_err(Error::git)?;
    let target_tree = target_commit.tree_id().map_err(Error::git)?;
    let workdir = repository.inner.workdir().ok_or(Error::MissingWorktree)?;
    let current_index = repository.inner.index_or_empty().map_err(Error::git)?;
    let current_paths = current_index
        .entries_with_paths_by_filter_map(|path, _| {
            std::str::from_utf8(path.as_bytes()).ok().map(PathBuf::from)
        })
        .map(|(_, path)| path)
        .collect::<Vec<_>>();

    let mut index = repository
        .inner
        .index_from_tree(&target_tree)
        .map_err(Error::git)?;
    let target_paths = index
        .entries_with_paths_by_filter_map(|path, _| {
            std::str::from_utf8(path.as_bytes()).ok().map(PathBuf::from)
        })
        .map(|(_, path)| path)
        .collect::<HashSet<_>>();
    let (mut index_state, index_path) = index.into_parts();
    let mut options = repository
        .inner
        .checkout_options(gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping)
        .map_err(Error::git)?;
    options.overwrite_existing = false;
    options.keep_going = false;
    let outcome = gix::worktree::state::checkout(
        &mut index_state,
        workdir,
        repository
            .inner
            .objects
            .clone()
            .into_arc()
            .map_err(Error::git)?,
        &gix::progress::Discard,
        &gix::progress::Discard,
        &AtomicBool::new(false),
        options,
    )
    .map_err(|error| Error::Checkout(error.to_string()))?;
    if !outcome.collisions.is_empty() || !outcome.errors.is_empty() {
        return Err(Error::Checkout(format!(
            "{} collision(s), {} file error(s)",
            outcome.collisions.len(),
            outcome.errors.len()
        )));
    }

    for path in current_paths {
        if !target_paths.contains(&path) {
            let path = workdir.join(path);
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Error::git(error)),
            }
        }
    }

    index = gix::index::File::from_state(index_state, index_path);
    index
        .write(gix::index::write::Options::default())
        .map_err(Error::git)?;

    let head = repository
        .inner
        .find_reference("HEAD")
        .map_err(Error::git)?;
    let previous_head = head.inner.target.clone();
    repository
        .inner
        .edit_reference(RefEdit {
            change: Change::Update {
                log: Default::default(),
                expected: PreviousValue::MustExistAndMatch(previous_head),
                new: Target::Symbolic(target_full_name),
            },
            name: FullName::try_from("HEAD").expect("HEAD is a valid reference"),
            deref: false,
        })
        .map_err(Error::git)?;

    Ok(BranchSwitchOutcome::Switched {
        from,
        to: name.as_str().to_owned(),
    })
}
