use super::*;

fn work(revision: u64) -> RecoveryCheckpointWork {
    RecoveryCheckpointWork {
        project_root: std::path::PathBuf::from("/tmp/h1-recovery-test"),
        session_id: 1,
        live_revision: revision,
        source_stage: "scene.usda".to_owned(),
        base_revision: None,
        stage_bytes: Vec::new(),
    }
}

#[test]
fn recovery_queue_is_bounded_and_keeps_latest_pending_revision() {
    let queue = RecoveryQueue::new();
    for revision in 1..=5 {
        queue.submit(work(revision)).expect("queue open");
    }
    let (pending, high_water, coalesced) = queue.stats();
    assert_eq!(pending, RECOVERY_QUEUE_CAPACITY as u64);
    assert_eq!(high_water, RECOVERY_QUEUE_CAPACITY as u64);
    assert_eq!(coalesced, 1);
    assert_eq!(queue.recv().expect("first pending work").live_revision, 2);
}

#[test]
fn recovery_result_backpressure_keeps_worker_result_sendable() {
    let (sender, receiver) = mpsc::sync_channel(RECOVERY_RESULT_CAPACITY);
    let result_backpressure = Arc::new(AtomicU64::new(0));
    let completed = Arc::new(AtomicU64::new(0));
    let full_reached = Arc::new(AtomicU64::new(0));
    let worker_backpressure = Arc::clone(&result_backpressure);
    let worker_completed = Arc::clone(&completed);
    let worker_full_reached = Arc::clone(&full_reached);
    let worker = std::thread::spawn(move || {
        for revision in 1..=(RECOVERY_RESULT_CAPACITY as u64 + 1) {
            let full_signal = Arc::clone(&worker_full_reached);
            assert!(send_recovery_result_with_hook(
                &sender,
                RecoveryResult {
                    session_id: 1,
                    live_revision: revision,
                    result: Ok(RecoveryMetadata {
                        format_version: 1,
                        session_id: 1,
                        live_revision: revision,
                        source_stage: "scene.usda".to_owned(),
                        base_revision: None,
                        stage_digest: format!("digest-{revision}"),
                        created_at_unix_ms: 0,
                    }),
                    worker_ms: 0.0,
                },
                &worker_backpressure,
                move || full_signal.store(revision, Ordering::Release),
            ));
            worker_completed.fetch_add(1, Ordering::Release);
        }
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while full_reached.load(Ordering::Acquire) != RECOVERY_RESULT_CAPACITY as u64 + 1
        && std::time::Instant::now() < deadline
    {
        std::thread::yield_now();
    }
    assert_eq!(
        full_reached.load(Ordering::Acquire),
        RECOVERY_RESULT_CAPACITY as u64 + 1,
        "worker must observe a full result queue before the drain"
    );

    for revision in 1..=RECOVERY_RESULT_CAPACITY as u64 {
        assert_eq!(
            receiver.recv().expect("recovery result").live_revision,
            revision
        );
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while completed.load(Ordering::Acquire) < RECOVERY_RESULT_CAPACITY as u64 + 1
        && std::time::Instant::now() < deadline
    {
        std::thread::yield_now();
    }
    assert_eq!(
        completed.load(Ordering::Acquire),
        RECOVERY_RESULT_CAPACITY as u64 + 1
    );
    assert_eq!(result_backpressure.load(Ordering::Acquire), 1);
    assert_eq!(
        receiver
            .recv()
            .expect("latest recovery result")
            .live_revision,
        RECOVERY_RESULT_CAPACITY as u64 + 1
    );
    worker.join().expect("recovery result sender remains alive");
}
