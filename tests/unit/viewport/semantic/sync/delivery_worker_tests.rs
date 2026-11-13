use std::collections::HashMap;

use tempfile::tempdir;
use usd_model::{HashDigest, SnapshotId, SnapshotSource};

use super::*;
use crate::project::cache::{ProjectCacheState, ProjectCacheStore, ProjectCacheTarget};
use crate::project::cache_hydration::ActiveProjectCacheContext;
use crate::project::catalog::manifest_store::ManifestStore;

fn work(revision: u64) -> DeliveryWork {
    DeliveryWork {
        identity: RuntimeDeliveryIdentity {
            session_id: 7,
            live_revision: LiveRevision(revision),
            projection_generation: 3,
        },
        project_root: PathBuf::from("/tmp/h1-delivery-test"),
        snapshot: SemanticSnapshot {
            snapshot_id: SnapshotId(format!("h1-{revision}")),
            source: SnapshotSource::Working {
                session: "h1-test".to_owned(),
                live_revision: revision,
            },
            config_hash: HashDigest::new([0; HashDigest::BYTE_LEN]),
            entities: HashMap::new(),
        }
        .into(),
        prepared_blobs: Vec::new(),
        prepared_runtime_payloads: PreparedRuntimePayloads::default(),
        profile: RuntimeProfile::NativeMedium,
        cache_context: None,
    }
}

#[test]
fn delivery_queue_is_bounded_and_keeps_latest_pending_revision() {
    let queue = DeliveryQueue::new();
    for revision in 1..=5 {
        assert!(queue.submit(work(revision)).is_ok());
    }
    let (pending, high_water, coalesced) = queue.stats();
    assert_eq!(pending, DELIVERY_QUEUE_CAPACITY as u64);
    assert_eq!(high_water, DELIVERY_QUEUE_CAPACITY as u64);
    assert_eq!(coalesced, 1);

    let mut revisions = Vec::new();
    for _ in 0..DELIVERY_QUEUE_CAPACITY {
        revisions.push(
            queue
                .pop()
                .expect("queued delivery")
                .identity
                .live_revision
                .0,
        );
    }
    assert_eq!(revisions, vec![2, 3, 4, 5]);
}

#[test]
fn delivery_result_backpressure_preserves_latest_completion() {
    let (sender, receiver) = mpsc::sync_channel(DELIVERY_RESULT_CAPACITY);
    let result_backpressure = Arc::new(AtomicU64::new(0));
    let completed = Arc::new(AtomicU64::new(0));
    let worker_backpressure = Arc::clone(&result_backpressure);
    let worker_completed = Arc::clone(&completed);
    let worker = std::thread::spawn(move || {
        for revision in 1..=(DELIVERY_RESULT_CAPACITY as u64 + 1) {
            assert!(send_delivery_result(
                &sender,
                DeliveryResult {
                    identity: RuntimeDeliveryIdentity {
                        session_id: 7,
                        live_revision: LiveRevision(revision),
                        projection_generation: 3,
                    },
                    bundle: Err(format!("test-{revision}")),
                    worker_ms: 0.0,
                    blob_reads: 0,
                    bytes: 0,
                    prepared_runtime_payloads: PreparedRuntimePayloads::default(),
                    cache_context: None,
                },
                &worker_backpressure,
            ));
            worker_completed.fetch_add(1, Ordering::Release);
        }
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while completed.load(Ordering::Acquire) < DELIVERY_RESULT_CAPACITY as u64
        && std::time::Instant::now() < deadline
    {
        std::thread::yield_now();
    }
    assert_eq!(
        completed.load(Ordering::Acquire),
        DELIVERY_RESULT_CAPACITY as u64,
        "worker must block on a full result queue, not drop the completion"
    );

    for revision in 1..=DELIVERY_RESULT_CAPACITY as u64 {
        assert_eq!(
            receiver
                .recv()
                .expect("runtime delivery result")
                .identity
                .live_revision
                .0,
            revision
        );
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while completed.load(Ordering::Acquire) < DELIVERY_RESULT_CAPACITY as u64 + 1
        && std::time::Instant::now() < deadline
    {
        std::thread::yield_now();
    }
    assert_eq!(
        completed.load(Ordering::Acquire),
        DELIVERY_RESULT_CAPACITY as u64 + 1
    );
    assert_eq!(result_backpressure.load(Ordering::Acquire), 1);
    assert_eq!(
        receiver
            .recv()
            .expect("latest runtime delivery result")
            .identity
            .live_revision
            .0,
        DELIVERY_RESULT_CAPACITY as u64 + 1
    );
    worker.join().expect("delivery result sender remains alive");
}

#[test]
fn complete_delivery_publishes_a_ready_descriptor_for_the_active_identity() {
    let project = tempdir().expect("temporary Project root");
    usd_git::Repository::init(project.path()).expect("Project repository");
    let manifest = usd_project::ProjectManifestV1::new(
        usd_project::ProjectId::new_v4(),
        "Delivery cache fixture",
        usd_project::ProjectRoot::Empty,
        Vec::new(),
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(project.path(), &manifest).expect("Project manifest");
    let context = ActiveProjectCacheContext::new(
        project.path().to_path_buf(),
        ProjectCacheTarget::ProjectRoot,
        RuntimeProfile::NativeMedium,
        usd_semantic::SemanticConfig::default().hash(),
    )
    .expect("cache identity");
    let mut work = work(9);
    work.project_root = project.path().to_path_buf();
    Arc::make_mut(&mut work.snapshot).config_hash = context.identity.config_hash;
    work.cache_context = Some(context.clone());

    let bundle = build_delivery(&work).expect("delivery bundle");
    let descriptor = ProjectCacheStore::new(project.path())
        .load(&context.identity)
        .expect("descriptor read")
        .expect("ready descriptor");
    assert_eq!(descriptor.state, ProjectCacheState::Ready);
    assert_eq!(descriptor.runtime, Some(bundle.manifest));
}
