use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use super::{
    CACHE_DIRECTORY, OUTBOX_DIRECTORY, PROJECT_METADATA_DIRECTORY, ProjectStageMutation,
    ProjectStageMutationQueue, ProjectWriteError, STAGE_MUTATION_CAPACITY,
};

pub(super) fn outbox_path(project_root: &Path) -> PathBuf {
    project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join(CACHE_DIRECTORY)
        .join(OUTBOX_DIRECTORY)
}

pub(super) fn submit_batch_locked_with_failure(
    project_root: &Path,
    mutations: &[ProjectStageMutation],
    fail_before_index: Option<usize>,
) -> Result<(), ProjectWriteError> {
    if mutations.is_empty() {
        return Ok(());
    }
    let path = outbox_path(project_root);
    let pending = read_pending(&path)?;
    if pending.len().saturating_add(mutations.len()) > STAGE_MUTATION_CAPACITY {
        return Err(busy_error());
    }
    fs::create_dir_all(&path).map_err(|_| filesystem_error())?;

    let mut temporary_paths = Vec::with_capacity(mutations.len());
    let mut final_paths = Vec::with_capacity(mutations.len());
    let result = (|| {
        for (index, mutation) in mutations.iter().enumerate() {
            if fail_before_index == Some(index) {
                return Err(filesystem_error());
            }
            let id = uuid::Uuid::new_v4();
            let temporary = path.join(format!(".{id}.tmp"));
            let final_path = path.join(format!("{id}.json"));
            let encoded = serde_json::to_vec(mutation).map_err(|_| filesystem_error())?;
            temporary_paths.push(temporary.clone());
            final_paths.push(final_path.clone());
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|_| filesystem_error())?;
            file.write_all(&encoded).map_err(|_| filesystem_error())?;
            file.sync_all().map_err(|_| filesystem_error())?;
        }
        for (temporary, final_path) in temporary_paths.iter().zip(&final_paths) {
            fs::rename(temporary, final_path).map_err(|_| filesystem_error())?;
        }
        Ok(())
    })();

    if result.is_err() {
        for temporary in &temporary_paths {
            let _ = fs::remove_file(temporary);
        }
        for final_path in &final_paths {
            let _ = fs::remove_file(final_path);
        }
    }
    result
}

#[cfg(test)]
fn take_test_failure(queue: &ProjectStageMutationQueue) -> Option<usize> {
    queue
        .test_fail_before_index
        .lock()
        .expect("Project stage mutation queue test hook is not poisoned")
        .take()
}

#[cfg(not(test))]
fn take_test_failure(_queue: &ProjectStageMutationQueue) -> Option<usize> {
    None
}

pub(super) fn take_failure(queue: &ProjectStageMutationQueue) -> Option<usize> {
    take_test_failure(queue)
}

pub(super) fn read_pending(
    path: &Path,
) -> Result<Vec<(PathBuf, ProjectStageMutation)>, ProjectWriteError> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(filesystem_error()),
    };
    let mut paths = entries
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|_| filesystem_error())
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "json")
    });
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path).map_err(|_| filesystem_error())?;
            let mutation = serde_json::from_slice(&bytes).map_err(|_| filesystem_error())?;
            Ok((path, mutation))
        })
        .collect()
}

fn busy_error() -> ProjectWriteError {
    ProjectWriteError::Failed {
        code: project_protocol::ProjectWriteErrorCode::Busy,
    }
}

pub(super) fn filesystem_error() -> ProjectWriteError {
    ProjectWriteError::Failed {
        code: project_protocol::ProjectWriteErrorCode::FilesystemFailure,
    }
}
