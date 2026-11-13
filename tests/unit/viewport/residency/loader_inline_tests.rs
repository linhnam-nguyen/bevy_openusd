use super::*;

fn job(key: u8, generation: u64) -> LoadJob<u8> {
    LoadJob {
        key,
        generation,
        cpu_bytes: u64::from(key),
        gpu_bytes: u64::from(key),
    }
}

#[test]
fn queue_is_bounded_and_coalesces_latest_metadata() {
    let mut queue = BoundedLoader::new(1);
    assert!(queue.enqueue(job(1, 1)));
    assert!(queue.enqueue(job(1, 2)));
    assert!(!queue.enqueue(job(2, 1)));
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.pop(), Some(job(1, 2)));
    assert!(queue.is_empty());
}

#[test]
fn cancellation_removes_pending_job_without_reordering_other_jobs() {
    let mut queue = BoundedLoader::new(3);
    assert!(queue.enqueue(job(1, 1)));
    assert!(queue.enqueue(job(2, 1)));
    assert_eq!(queue.cancel(&1), Some(job(1, 1)));
    assert_eq!(queue.pop().map(|job| job.key), Some(2));
    assert!(queue.is_empty());
}

#[test]
fn cancellation_reenqueue_stays_bounded_and_moves_key_to_current_tail() {
    let capacity = 3;
    let mut queue = BoundedLoader::new(capacity);
    assert!(queue.enqueue(job(1, 0)));

    let cycles = capacity * 128;
    for generation in 1..=cycles {
        let generation = generation as u64;
        assert_eq!(
            queue.cancel(&1).map(|job| job.generation),
            Some(generation - 1)
        );
        assert!(queue.enqueue(job(1, generation)));
        assert_eq!(queue.order.len(), queue.jobs.len());
        assert!(queue.order.len() <= capacity);
    }

    assert!(queue.enqueue(job(2, 1)));
    assert!(queue.enqueue(job(3, 1)));
    assert_eq!(queue.cancel(&2).map(|job| job.generation), Some(1));
    assert!(queue.enqueue(job(2, 99)));
    assert_eq!(queue.order.len(), capacity);
    assert_eq!(
        queue.pop().map(|job| (job.key, job.generation)),
        Some((1, cycles as u64))
    );
    assert_eq!(queue.pop().map(|job| job.key), Some(3));
    assert_eq!(
        queue.pop().map(|job| (job.key, job.generation)),
        Some((2, 99))
    );
    assert!(queue.is_empty());
    assert!(queue.order.is_empty());
}
