use super::*;

fn ray() -> Ray3d {
    Ray3d::new(Vec3::ZERO, Dir3::NEG_Z)
}

fn hit(logical_id: i64) -> GpuPointPickResult {
    GpuPointPickResult {
        hit: Some(GpuPointHit {
            instancer: Arc::from("/Equipment/Instances"),
            logical_id,
            distance: 1.0,
        }),
        visible_rows: 1,
    }
}

#[test]
fn late_point_readback_cannot_replace_latest_pointer_result() {
    let picker = GpuPointPicker::default();
    let old = picker.request(ray());
    assert_eq!(picker.start().unwrap().sequence, old);

    let skipped = picker.request(ray());
    let current = picker.request(ray());
    assert!(picker.start().is_none());
    picker.complete(old, Ok(hit(17)));
    assert!(picker.take_result(old).is_none());
    assert!(picker.take_result(skipped).is_none());
    assert_eq!(picker.start().unwrap().sequence, current);

    picker.complete(current, Ok(hit(0)));
    assert!(picker.take_result(old).is_none());
    assert_eq!(
        picker
            .take_result(current)
            .unwrap()
            .unwrap()
            .hit
            .unwrap()
            .logical_id,
        0
    );
}

#[test]
fn representation_change_discards_in_flight_point_readback() {
    let picker = GpuPointPicker::default();
    let old = picker.request(ray());
    assert_eq!(picker.start().unwrap().sequence, old);
    let revision = picker.representation_revision();

    picker.invalidate_representation(false);
    assert_eq!(picker.representation_revision(), revision + 1);
    assert_eq!(picker.view_revision(), 1);
    picker.complete(old, Ok(hit(17)));
    assert!(picker.take_result(old).is_none());
    assert!(picker.start().is_none());

    let promoted = picker.request(ray());
    assert_eq!(picker.start().unwrap().sequence, promoted);
    picker.invalidate_representation(true);
    assert_eq!(picker.representation_revision(), revision + 2);
    assert_eq!(picker.view_revision(), 1);
    picker.complete(promoted, Ok(hit(17)));
    assert!(picker.take_result(promoted).is_none());

    let current = picker.request(ray());
    assert_eq!(picker.start().unwrap().sequence, current);
    picker.complete(current, Ok(hit(17)));
    assert_eq!(
        picker
            .take_result(current)
            .unwrap()
            .unwrap()
            .hit
            .unwrap()
            .logical_id,
        17
    );
}

#[test]
fn draw_budget_is_partitioned_even_when_one_mesh_exceeds_work_budget() {
    let triangles = 262_144usize;
    let rows = 4_096usize;
    // One mesh/chunk requires 1,073,741,824 triangle-row evaluations, but
    // every submitted task must remain below MAX_TRIANGLE_WORK.
    let mut offset = 0usize;
    let mut batches = 0usize;
    while offset < rows {
        let limit = bounded_pick_rows(rows, offset, triangles, MAX_TRIANGLE_WORK);
        assert!(limit > 0);
        assert!(limit * triangles <= MAX_TRIANGLE_WORK);
        offset += limit;
        batches += 1;
    }
    assert_eq!(offset, rows);
    assert_eq!(batches, 64);
}

#[test]
fn later_batch_hit_is_nearest_and_visible_count_deduplicates_parts() {
    let request = PickRequest {
        sequence: 1,
        ray: ray(),
    };
    let mut traversal = PickTraversal {
        request,
        draws: Vec::new(),
        draw_cursor: 0,
        visible_cursor: 0,
        nearest: None,
        visible_chunks: HashMap::default(),
        waiting: None,
    };
    let first = GpuPointHit {
        instancer: Arc::from("/Equipment/Instances"),
        logical_id: 10,
        distance: 5.0,
    };
    let second = GpuPointHit {
        instancer: Arc::from("/Equipment/Instances"),
        logical_id: 0,
        distance: 2.0,
    };
    traversal.add_batch(BatchReport {
        hit: Some(first),
        visible_chunks: vec![(4, 4), (4, 4)],
    });
    traversal.add_batch(BatchReport {
        hit: Some(second),
        visible_chunks: vec![(4, 4), (5, 2)],
    });
    let result = traversal.result();
    assert_eq!(result.hit.unwrap().logical_id, 0);
    assert_eq!(result.visible_rows, 6);
}

#[test]
fn cancelled_traversal_never_overwrites_new_request() {
    let picker = GpuPointPicker::default();
    let old = picker.request(ray());
    assert!(picker.start().is_some());
    let new = picker.request(ray());
    assert!(!picker.is_current(old));
    picker.cancel_in_flight();
    assert_eq!(picker.start().unwrap().sequence, new);
    picker.complete(new, Ok(hit(0)));
    assert_eq!(
        picker
            .take_result(new)
            .unwrap()
            .unwrap()
            .hit
            .unwrap()
            .logical_id,
        0
    );
}

#[test]
fn over_capacity_multipart_draws_are_split_into_bounded_tasks() {
    // 4,097 rows; two prototype parts; user-configured chunk cap 8:
    // 513 chunks * 2 = 1,026 draw descriptions (> MAX_DRAWS).
    let draws = 4_097usize.div_ceil(8) * 2;
    assert!(draws > MAX_DRAWS);
    let first_batch = draws.min(MAX_DRAWS);
    let second_batch = draws - first_batch;
    assert_eq!((first_batch, second_batch), (1024, 2));
    assert!(bounded_pick_rows(8, 0, 1, MAX_TRIANGLE_WORK) > 0);
}

#[test]
fn workgroup_limit_cannot_overflow_dispatch_dimension() {
    assert_eq!(
        bounded_pick_rows(20_000_000, 0, 1, MAX_TRIANGLE_WORK),
        MAX_ROWS_PER_TASK
    );
    assert_eq!(MAX_ROWS_PER_TASK.div_ceil(64), 65_535);
}

#[test]
fn readback_failure_is_distinct_from_a_geometric_miss() {
    let picker = GpuPointPicker::default();
    let request = picker.request(ray());
    assert_eq!(picker.start().unwrap().sequence, request);
    picker.complete(request, Err(GpuPointPickError::ReadbackFailed));
    assert!(matches!(
        picker.take_result(request),
        Some(Err(GpuPointPickError::ReadbackFailed))
    ));
    assert!(picker.take_result(request).is_none());
}
