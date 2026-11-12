//! Shared test support utilities for root project service tests.
//!
//! This module contains reusable helpers that are shared across multiple
//! project-service test modules and operate through public/external surfaces.
//! Private white-box helpers remain local to their owning unit-test modules.

use std::fs;
use std::path::Path;
use std::process::Command;

use usd_project::ProjectSummary;

use crate::project::service::ProjectApplicationService;

/// Query the configured Git default initial branch, falling back to "main".
pub(super) fn configured_initial_branch() -> String {
    let directory = tempfile::tempdir().expect("create temporary directory for Git config");
    let output = Command::new("git")
        .args(["config", "--default", "main", "init.defaultBranch"])
        .current_dir(directory.path())
        .output()
        .expect("read configured Git default branch");
    assert!(
        output.status.success(),
        "read configured Git default branch: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("Git default branch is UTF-8")
        .trim()
        .to_owned()
}

/// Execute a Git command in the given directory and assert that it succeeded.
pub(super) fn run_git(directory: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run git command");
    assert!(
        output.status.success(),
        "git command failed: {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Create a temporary workspace and project for service-level testing.
pub(super) fn service_with_project(
    directory: &Path,
) -> (ProjectApplicationService, ProjectSummary) {
    let parent = directory.join("projects");
    fs::create_dir(&parent).unwrap();
    let mut service = ProjectApplicationService::open(directory.join("workspace.json")).unwrap();
    let project = service.create_project(&parent, "Project").unwrap();
    (service, project)
}
