use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use gix::object::tree::EntryKind;

use crate::{
    CommitInfo, CommitRequest, CommitSignature, Error, MaterializedRevision, Result, Revision,
    RevisionId, RevisionSpec,
};

/// The Git boundary used by the rest of USDHub.
pub trait GitRepository {
    fn resolve_revision(&self, spec: &RevisionSpec) -> Result<Revision>;

    fn head(&self) -> Result<Option<Revision>>;

    fn read_commit(&self, id: &RevisionId) -> Result<CommitInfo>;

    fn materialize_revision(
        &self,
        id: &RevisionId,
        destination: &Path,
    ) -> Result<MaterializedRevision>;

    fn create_commit(&mut self, request: CommitRequest) -> Result<RevisionId>;
}

/// A repository opened through the private `gix` implementation.
pub struct Repository {
    inner: gix::Repository,
}

impl Repository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let inner = gix::open(path.as_ref()).map_err(Error::git)?;
        Ok(Self { inner })
    }

    fn commit_for_spec(&self, spec: &str) -> Result<gix::Commit<'_>> {
        let id = self.inner.rev_parse_single(spec).map_err(Error::git)?;
        self.inner.find_commit(id.detach()).map_err(Error::git)
    }

    fn commit_for_id(&self, id: &RevisionId) -> Result<gix::Commit<'_>> {
        self.commit_for_spec(id.as_str())
    }

    fn revision_from_commit(commit: &gix::Commit<'_>) -> Revision {
        Revision::from_id(RevisionId::new(commit.id().to_string()))
    }

    fn commit_info(&self, commit: gix::Commit<'_>) -> Result<CommitInfo> {
        let id = RevisionId::new(commit.id().to_string());
        let decoded = commit.decode().map_err(Error::git)?;
        let author = signature(decoded.author().map_err(Error::git)?)?;
        let committer = signature(decoded.committer().map_err(Error::git)?)?;
        let message = String::from_utf8_lossy(decoded.message.as_ref()).into_owned();
        let parents = decoded
            .parents()
            .map(|parent| RevisionId::new(parent.to_string()))
            .collect();
        let tree_id = RevisionId::new(decoded.tree().to_string());

        Ok(CommitInfo {
            id,
            tree_id,
            parents,
            author,
            committer,
            message,
        })
    }
}

fn signature(value: gix::actor::SignatureRef<'_>) -> Result<CommitSignature> {
    let actor = value.actor();
    let time = value.time().map_err(Error::git)?;
    Ok(CommitSignature {
        name: String::from_utf8_lossy(actor.name.as_ref()).into_owned(),
        email: String::from_utf8_lossy(actor.email.as_ref()).into_owned(),
        time_seconds: time.seconds,
        time_offset_seconds: time.offset,
    })
}

impl GitRepository for Repository {
    fn resolve_revision(&self, spec: &RevisionSpec) -> Result<Revision> {
        if spec.as_str().is_empty() {
            return Err(Error::InvalidRevisionSpec(String::new()));
        }
        Ok(Self::revision_from_commit(
            &self.commit_for_spec(spec.as_str())?,
        ))
    }

    fn head(&self) -> Result<Option<Revision>> {
        match self.inner.head_commit() {
            Ok(commit) => Ok(Some(Self::revision_from_commit(&commit))),
            Err(error) => {
                let is_unborn = self.inner.head_ref().map_err(Error::git)?.is_none();
                if is_unborn {
                    Ok(None)
                } else {
                    Err(Error::git(error))
                }
            }
        }
    }

    fn read_commit(&self, id: &RevisionId) -> Result<CommitInfo> {
        self.commit_info(self.commit_for_id(id)?)
    }

    fn materialize_revision(
        &self,
        id: &RevisionId,
        destination: &Path,
    ) -> Result<MaterializedRevision> {
        ensure_destination_is_empty(destination)?;
        fs::create_dir_all(destination)?;

        let commit = self.commit_for_id(id)?;
        let tree = commit.tree().map_err(Error::git)?;
        let mut file_count = 0;
        materialize_tree(&tree, destination, Path::new(""), &mut file_count)?;

        Ok(MaterializedRevision {
            revision: id.clone(),
            root: destination.to_path_buf(),
            file_count,
        })
    }

    fn create_commit(&mut self, request: CommitRequest) -> Result<RevisionId> {
        if request.message.trim().is_empty() {
            return Err(Error::Git("commit message must not be empty".to_owned()));
        }
        if !request.source_directory.is_dir() {
            return Err(Error::InvalidSourceDirectory(request.source_directory));
        }

        let message = if request.message.ends_with('\n') {
            request.message
        } else {
            format!("{}\n", request.message)
        };
        let tree_id = write_source_tree(&self.inner, &request.source_directory)?;
        let parent = self
            .inner
            .head_commit()
            .ok()
            .map(|commit| commit.id().detach());
        let parents = parent.into_iter();
        let commit_id = self
            .inner
            .commit("HEAD", message, tree_id, parents)
            .map_err(Error::git)?;
        Ok(RevisionId::new(commit_id.to_string()))
    }
}

fn write_source_tree(repository: &gix::Repository, source: &Path) -> Result<gix::hash::ObjectId> {
    let mut entries = fs::read_dir(source)
        .map_err(Error::from)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    let mut editor = repository.empty_tree().edit().map_err(Error::git)?;
    for entry in entries {
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let relative = PathBuf::from(name.clone());
        let path = entry.path();
        let file_type = entry.file_type().map_err(Error::from)?;
        let (kind, object_id) = if file_type.is_dir() {
            (EntryKind::Tree, write_source_tree(repository, &path)?)
        } else if file_type.is_file() {
            let data = fs::read(&path).map_err(Error::from)?;
            (
                EntryKind::Blob,
                repository.write_blob(data).map_err(Error::git)?.detach(),
            )
        } else {
            return Err(Error::UnsupportedSourceEntry {
                path,
                kind: "non-regular file".to_owned(),
            });
        };
        let name = name
            .to_str()
            .ok_or_else(|| Error::InvalidPath(relative.clone()))?;
        editor.upsert(name, kind, object_id).map_err(Error::git)?;
    }
    Ok(editor.write().map_err(Error::git)?.detach())
}

fn ensure_destination_is_empty(destination: &Path) -> Result<()> {
    if !destination.exists() {
        return Ok(());
    }

    let mut entries = fs::read_dir(destination)?;
    if entries.next().transpose()?.is_some() {
        return Err(Error::DestinationNotEmpty(destination.to_path_buf()));
    }
    Ok(())
}

fn materialize_tree(
    tree: &gix::Tree<'_>,
    destination: &Path,
    relative: &Path,
    file_count: &mut usize,
) -> Result<()> {
    for entry in tree.iter() {
        let entry = entry.map_err(Error::git)?;
        let filename = entry.filename().to_owned();
        let filename = std::str::from_utf8(filename.as_ref())
            .map_err(|_| Error::InvalidPath(relative.join("<non-utf8>")))?;
        let relative_path = relative.join(filename);
        validate_relative_path(&relative_path)?;

        match entry.kind() {
            EntryKind::Tree => {
                let child_destination = destination.join(&relative_path);
                fs::create_dir_all(&child_destination)?;
                let child_tree = entry.object().map_err(Error::git)?.into_tree();
                materialize_tree(&child_tree, destination, &relative_path, file_count)?;
            }
            EntryKind::Blob | EntryKind::BlobExecutable | EntryKind::Link => {
                let file_destination = destination.join(&relative_path);
                if let Some(parent) = file_destination.parent() {
                    fs::create_dir_all(parent)?;
                }
                let blob = entry.object().map_err(Error::git)?.into_blob();
                fs::write(file_destination, blob.data.clone())?;
                *file_count += 1;
            }
            EntryKind::Commit => {
                return Err(Error::UnsupportedEntry {
                    path: relative_path,
                    kind: "submodule".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<()> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(Error::InvalidPath(path.to_path_buf()));
    }
    Ok(())
}
