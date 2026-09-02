use std::{
    cell::Cell,
    collections::HashMap,
    fs,
    path::Path,
    process::{Command, Output},
};

use super::*;
use openusd::usd::Stage;
use usd_git::CommitSignature;
use usd_model::EntityKey;

fn commit(id: &str, parents: &[&str]) -> CommitInfo {
    CommitInfo {
        id: RevisionId::new(id),
        tree_id: RevisionId::new(format!("tree-{id}")),
        parents: parents
            .iter()
            .map(|parent| RevisionId::new(*parent))
            .collect(),
        author: CommitSignature {
            name: format!("Author {id}"),
            email: format!("{id}@example.test"),
            time_seconds: 1,
            time_offset_seconds: 0,
        },
        committer: CommitSignature {
            name: format!("Committer {id}"),
            email: format!("{id}@example.test"),
            time_seconds: 1,
            time_offset_seconds: 0,
        },
        message: format!("commit {id}"),
    }
}

#[test]
fn returns_the_most_recent_commit_that_changed_the_property() {
    let history = vec![
        commit("c10", &["c9"]),
        commit("c9", &["c8"]),
        commit("c8", &["c7"]),
        commit("c7", &["c6"]),
        commit("c6", &[]),
    ];
    let values = HashMap::from([
        ("c10", Some(CanonicalValue::Text("door".to_owned()))),
        ("c9", Some(CanonicalValue::Text("door".to_owned()))),
        ("c8", Some(CanonicalValue::Text("door".to_owned()))),
        ("c7", Some(CanonicalValue::Text("wall".to_owned()))),
        ("c6", Some(CanonicalValue::Text("wall".to_owned()))),
    ]);
    let change = find_last_property_change(&history, |revision| {
        Ok(values.get(revision.as_str()).cloned().flatten())
    })
    .expect("history lookup succeeds")
    .expect("property has a committed change");

    assert_eq!(change.0.id.as_str(), "c8");
    assert_eq!(change.1, Some(CanonicalValue::Text("wall".to_owned())));
    assert_eq!(change.2, Some(CanonicalValue::Text("door".to_owned())));
}

#[test]
fn root_commit_addition_is_a_property_change() {
    let history = vec![commit("c1", &[])];
    let change = find_last_property_change(&history, |revision| {
        Ok((revision.as_str() == "c1").then_some(CanonicalValue::Integer(1)))
    })
    .expect("history lookup succeeds")
    .expect("root addition is found");

    assert_eq!(change.0.id.as_str(), "c1");
    assert!(change.1.is_none());
    assert_eq!(change.2, Some(CanonicalValue::Integer(1)));
}

#[test]
fn worker_resolves_the_older_property_change_from_real_git_history() {
    let repository_dir = tempfile::tempdir().expect("create provenance repository");
    configure_repository(repository_dir.path());
    write_stage(repository_dir.path(), "wall", "Initial Mark");
    commit_repository(repository_dir.path(), "initial BIM property");
    write_stage(repository_dir.path(), "door", "Change Mark");
    commit_repository(repository_dir.path(), "change BIM property");
    write_stage(repository_dir.path(), "door", "Unrelated later change");
    commit_repository(repository_dir.path(), "unrelated later change");

    let repository = Repository::open(repository_dir.path()).expect("open provenance repository");
    let head = repository
        .head()
        .expect("read repository head")
        .expect("repository has a head");
    let expected_change = repository
        .history(head.id(), MAX_PROVENANCE_HISTORY)
        .expect("read provenance history")
        .get(1)
        .expect("history contains the property-changing commit")
        .id
        .clone();
    let job = BimProvenanceJob {
        request_id: "provenance-1".to_owned(),
        target: SceneAnchor::active_session("/World/Door"),
        property: "bim:Mark".to_owned(),
        entity_key: EntityKey::from("/World/Door"),
        history_head: head.id().clone(),
        stage_path: repository_dir.path().join("model.usda"),
        activation_generation: 0,
        generation: 1,
    };

    let provenance = match resolve_job(&job, || true) {
        ResolveOutcome::Completed(Ok(provenance)) => provenance,
        ResolveOutcome::Completed(Err(error)) => {
            panic!("worker should resolve the real provenance fixture: {error}")
        }
        ResolveOutcome::Cancelled => panic!("provenance fixture unexpectedly cancelled"),
    };
    assert_eq!(
        provenance.commit_id.as_deref(),
        Some(expected_change.as_str())
    );
    assert_eq!(
        provenance.commit_message.as_deref(),
        Some("change BIM property\n")
    );
    assert_eq!(
        provenance.old_value,
        Some(CanonicalValue::Text("wall".to_owned()))
    );
    assert_eq!(
        provenance.new_value,
        Some(CanonicalValue::Text("door".to_owned()))
    );
    assert_eq!(provenance.history_head, head.id().to_string());
}

#[test]
fn worker_cancels_a_started_job_when_its_generation_is_superseded() {
    let repository_dir = tempfile::tempdir().expect("create provenance repository");
    configure_repository(repository_dir.path());
    write_stage(repository_dir.path(), "wall", "Initial Mark");
    commit_repository(repository_dir.path(), "initial BIM property");
    write_stage(repository_dir.path(), "door", "Change Mark");
    commit_repository(repository_dir.path(), "change BIM property");

    let repository = Repository::open(repository_dir.path()).expect("open provenance repository");
    let head = repository
        .head()
        .expect("read repository head")
        .expect("repository has a head");
    let job = BimProvenanceJob {
        request_id: "provenance-stale".to_owned(),
        target: SceneAnchor::active_session("/World/Door"),
        property: "bim:Mark".to_owned(),
        entity_key: EntityKey::from("/World/Door"),
        history_head: head.id().clone(),
        stage_path: repository_dir.path().join("model.usda"),
        activation_generation: 0,
        generation: 1,
    };
    let first_check = Cell::new(true);

    let outcome = resolve_job(&job, || first_check.replace(false));

    assert!(matches!(outcome, ResolveOutcome::Cancelled));
}

fn configure_repository(directory: &Path) {
    run_git(directory, ["init", "-b", "main"]);
    run_git(directory, ["config", "user.name", "USDHub Test"]);
    run_git(directory, ["config", "user.email", "test@usdhub.invalid"]);
}

fn write_stage(directory: &Path, mark: &str, display_name: &str) {
    let stage = format!(
        "#usda 1.0\n(\n    defaultPrim = \"World\"\n)\n\ndef Xform \"World\" {{\n    def Xform \"Door\" {{\n        custom string bim:Mark = \"{mark}\"\n        custom string bim:DisplayName = \"{display_name}\"\n    }}\n}}\n"
    );
    fs::write(directory.join("model.usda"), stage).expect("write provenance stage");
    Stage::open(
        directory
            .join("model.usda")
            .to_str()
            .expect("provenance stage path is UTF-8"),
    )
    .expect("provenance stage parses");
}

fn commit_repository(directory: &Path, message: &str) {
    run_git(directory, ["add", "model.usda"]);
    run_git(directory, ["commit", "-m", message]);
}

fn run_git<const N: usize>(directory: &Path, args: [&str; N]) -> Output {
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
    output
}
