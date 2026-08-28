use std::sync::atomic::AtomicBool;

use gix::refs::transaction::{Change, PreviousValue, RefEdit};
use gix::refs::{FullName, Target};

use crate::{Error, Result};

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

    let mut index = repository
        .inner
        .index_from_tree(&target_tree)
        .map_err(Error::git)?;
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
