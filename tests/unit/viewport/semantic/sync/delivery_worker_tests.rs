use std::collections::HashMap;

use tempfile::tempdir;
use usd_model::{HashDigest, SnapshotId, SnapshotSource};

use super::*;
use crate::cache::hydration::ActiveProjectCacheContext;
use crate::cache::{ProjectCacheState, ProjectCacheStore, ProjectCacheTarget};
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
        cache_provenance: None,
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
                    cache_provenance: None,
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
    work.cache_provenance = DeliveryCacheProvenance::capture(&context, None);
    assert!(work.cache_provenance.is_some());
    work.cache_context = Some(context.clone());

    let bundle = build_delivery(&work).expect("delivery bundle");
    let descriptor = ProjectCacheStore::new(project.path())
        .load(&context.identity)
        .expect("descriptor read")
        .expect("ready descriptor");
    assert_eq!(descriptor.state, ProjectCacheState::Ready);
    assert_eq!(descriptor.runtime, Some(bundle.manifest));
}

fn scene_delivery_fixture(
    name: &str,
) -> (
    tempfile::TempDir,
    usd_project::SceneId,
    PathBuf,
    ActiveProjectCacheContext,
    usd_bevy::LiveStage,
) {
    let project = tempdir().expect("temporary Project root");
    usd_git::Repository::init(project.path()).expect("Project repository");
    let scene_id = usd_project::SceneId::new_v4();
    let manifest = usd_project::ProjectManifestV1::new(
        usd_project::ProjectId::new_v4(),
        name,
        usd_project::ProjectRoot::Scene(scene_id),
        vec![usd_project::SceneManifestEntry {
            id: scene_id,
            storage_key: usd_project::StorageKey::new("root").expect("storage key"),
            display_name: "Root".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(project.path(), &manifest).expect("Project manifest");
    let scene_file =
        crate::project::scene::authoring::author_scene_atomic(project.path(), scene_id)
            .expect("author root Scene");
    let live = usd_bevy::LiveStage::open_document(&scene_file).expect("open sealed LiveStage");
    let context = ActiveProjectCacheContext::new(
        project.path().to_path_buf(),
        ProjectCacheTarget::ProjectRoot,
        RuntimeProfile::NativeMedium,
        usd_semantic::SemanticConfig::default().hash(),
    )
    .expect("cache identity");
    (project, scene_id, scene_file, context, live)
}

#[test]
fn superseded_generation_prevents_older_delivery_from_publishing_or_advancing() {
    let (project, _scene_id, _scene_file, context, live) =
        scene_delivery_fixture("Superseded Delivery");
    let store = ProjectCacheStore::new(project.path());
    let provenance = DeliveryCacheProvenance::capture(&context, Some(&live))
        .expect("capture initial delivery provenance");
    assert_eq!(provenance.generation, 1);

    // Concurrent mutation advances the desired generation before delivery finishes.
    let newer_generation = store
        .advance_generation(&context.identity.target, context.identity.config_hash)
        .expect("advance newer desired generation");
    assert!(newer_generation > provenance.generation);

    let mut work = work(10);
    work.project_root = project.path().to_path_buf();
    Arc::make_mut(&mut work.snapshot).config_hash = context.identity.config_hash;
    work.cache_context = Some(context.clone());
    work.cache_provenance = Some(provenance);

    let _bundle = build_delivery(&work).expect("runtime delivery still succeeds");
    assert!(
        store
            .load(&context.identity)
            .expect("read descriptor")
            .is_none(),
        "superseded delivery must not publish Ready"
    );
    let pending = crate::cache::sync::PendingIntentStore::new(project.path())
        .load(&context.identity.target)
        .expect("read pending intent")
        .expect("newer desired obligation survives");
    assert_eq!(
        pending.generation, newer_generation,
        "older delivery must not advance or displace newer desired generation"
    );
}

#[test]
fn retired_target_prevents_in_flight_delivery_from_resurrecting_ready() {
    let (project, _scene_id, _scene_file, context, live) =
        scene_delivery_fixture("Retired Delivery");
    let store = ProjectCacheStore::new(project.path());
    let provenance = DeliveryCacheProvenance::capture(&context, Some(&live))
        .expect("capture delivery provenance");

    // Target is retired while delivery is in flight.
    store
        .retire_target(&context.identity.target)
        .expect("retire managed target");

    let mut work = work(11);
    work.project_root = project.path().to_path_buf();
    Arc::make_mut(&mut work.snapshot).config_hash = context.identity.config_hash;
    work.cache_context = Some(context.clone());
    work.cache_provenance = Some(provenance);

    let _bundle = build_delivery(&work).expect("runtime delivery still succeeds");
    assert!(
        store
            .load(&context.identity)
            .expect("read descriptor")
            .is_none(),
        "retired target must not be resurrected as Ready"
    );
}

#[test]
fn source_mutation_between_capture_and_publish_declines_stale_ready_while_returning_bundle() {
    let (project, _scene_id, scene_file, context, live) =
        scene_delivery_fixture("Mutated Delivery");
    let store = ProjectCacheStore::new(project.path());
    let provenance = DeliveryCacheProvenance::capture(&context, Some(&live))
        .expect("capture delivery provenance");

    // Mutate canonical source on disk after capture and before delivery publishes.
    std::fs::write(
        &scene_file,
        b"#usda 1.0\n(\n    defaultPrim = \"SceneRoot\"\n)\ndef Xform \"SceneRoot\" {\n    string marker = \"mutated-after-capture\"\n}\n",
    )
    .expect("mutate canonical scene");

    let mut work = work(12);
    work.project_root = project.path().to_path_buf();
    Arc::make_mut(&mut work.snapshot).config_hash = context.identity.config_hash;
    work.cache_context = Some(context.clone());
    work.cache_provenance = Some(provenance);

    let _bundle = build_delivery(&work).expect("runtime delivery still succeeds");
    assert!(
        store
            .load(&context.identity)
            .expect("read descriptor")
            .is_none(),
        "delivery captured from source A must not publish Ready after source mutates to B"
    );
    let updated_identity = crate::cache::ProjectCacheIdentity::for_project(
        project.path(),
        context.identity.target.clone(),
        context.identity.profile,
        context.identity.config_hash,
    )
    .expect("compute updated identity");
    assert!(
        store
            .load(&updated_identity)
            .expect("read updated descriptor")
            .is_none(),
        "source A payload must not be mislabeled under source B identity"
    );
}

#[test]
fn unsaved_live_stage_edits_decline_persistent_delivery_provenance() {
    let (_project, _scene_id, _scene_file, context, live) =
        scene_delivery_fixture("Unsaved Delivery");
    live.stage
        .define_prim("/SceneRoot/UnsavedTransient")
        .expect("author transient prim in memory");
    assert_ne!(live.authoring_generation(), LiveRevision::default());
    assert!(
        DeliveryCacheProvenance::capture(&context, Some(&live)).is_none(),
        "unsaved live-stage edits must not qualify for persistent canonical Ready publication"
    );
}
