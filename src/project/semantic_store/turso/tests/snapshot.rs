use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use usd_model::{SnapshotId, SnapshotSource};

use super::super::TursoSemanticStore;
use super::{runtime, snapshot};
use crate::project::semantic_store::{SemanticStore, get_or_regenerate_commit_snapshot};

#[test]
fn committed_snapshot_is_immutable_and_git_aliases_are_stable() {
    runtime().block_on(async {
        let mut store = TursoSemanticStore::open_memory()
            .await
            .expect("durable store opens");
        let first = snapshot("commit-a", "snapshot-a", "A", 1);
        store
            .put_snapshot(&first)
            .await
            .expect("first snapshot persists");
        store
            .put_snapshot(&first)
            .await
            .expect("idempotent snapshot write succeeds");

        let conflicting = snapshot("commit-a", "snapshot-a", "B", 2);
        assert!(store.put_snapshot(&conflicting).await.is_err());
        assert_eq!(
            store
                .get_snapshot(&first.snapshot_id)
                .await
                .expect("snapshot reads")
                .expect("snapshot remains present"),
            first
        );

        let same_content_other_commit = snapshot("commit-b", "snapshot-a", "A", 1);
        store
            .put_snapshot(&same_content_other_commit)
            .await
            .expect("same content can be aliased to another commit");
        let aliased = store
            .get_commit_snapshot("commit-b")
            .await
            .expect("commit alias reads")
            .expect("commit alias exists");
        assert_eq!(
            aliased.source,
            SnapshotSource::GitCommit {
                oid: "commit-b".to_owned()
            }
        );
        assert_eq!(aliased.snapshot_id, first.snapshot_id);
    });
}

#[test]
fn cache_miss_regeneration_is_persisted() {
    runtime().block_on(async {
        let mut store = TursoSemanticStore::open_memory()
            .await
            .expect("durable store opens");
        let calls = Arc::new(AtomicUsize::new(0));
        let first_calls = Arc::clone(&calls);
        let first = get_or_regenerate_commit_snapshot(&mut store, "commit-a", || async move {
            first_calls.fetch_add(1, Ordering::SeqCst);
            Ok(snapshot("commit-a", "snapshot-a", "A", 1))
        })
        .await
        .expect("cache miss regenerates");
        assert_eq!(first.snapshot_id, SnapshotId("snapshot-a".to_owned()));

        let second = get_or_regenerate_commit_snapshot(&mut store, "commit-a", || async {
            Err(anyhow::anyhow!("regenerator must not run on cache hit"))
        })
        .await
        .expect("cache hit reads persisted snapshot");
        assert_eq!(second, first);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn cached_commits_can_be_diffed_without_bevy() {
    runtime().block_on(async {
        let mut store = TursoSemanticStore::open_memory()
            .await
            .expect("durable store opens");
        let baseline = snapshot("commit-a", "snapshot-a", "A", 1);
        let current = snapshot("commit-b", "snapshot-b", "B", 2);
        store
            .put_snapshot(&baseline)
            .await
            .expect("baseline persists");
        store
            .put_snapshot(&current)
            .await
            .expect("current persists");

        let loaded_baseline = store
            .get_commit_snapshot("commit-a")
            .await
            .expect("baseline reads")
            .expect("baseline exists");
        let loaded_current = store
            .get_commit_snapshot("commit-b")
            .await
            .expect("current reads")
            .expect("current exists");
        let diff = usd_diff::compare(&loaded_baseline, &loaded_current);
        assert_eq!(diff.summary.changed, 1);
        assert_eq!(diff.summary.metadata, 1);
    });
}
