use super::{FrameReadbackCorrelation, MAX_PENDING_READBACK_TRACES, unpack_rgba_readback};
use bevy::render::renderer::RenderDevice;
use viewport_streaming::FrameTransportMetrics;

#[test]
fn render_identity_survives_asynchronous_readback_completion() {
    let metrics = FrameTransportMetrics::default();
    let mut correlation = FrameReadbackCorrelation::new(metrics);

    correlation.push_render_trace();
    correlation.push_render_trace();

    let first = correlation
        .take_readback_trace()
        .expect("first render trace must match first readback");
    let second = correlation
        .take_readback_trace()
        .expect("second render trace must match second readback");

    assert_eq!(first.sequence, 1);
    assert_eq!(second.sequence, 2);
    assert!(first.readback_timestamp_ns.is_some());
    assert!(second.readback_timestamp_ns.is_some());
}

#[test]
fn correlation_overflow_is_bounded_and_fails_closed() {
    let metrics = FrameTransportMetrics::default();
    let mut correlation = FrameReadbackCorrelation::new(metrics.clone());

    for _ in 0..=MAX_PENDING_READBACK_TRACES {
        correlation.push_render_trace();
    }

    assert!(correlation.pending.len() <= MAX_PENDING_READBACK_TRACES);
    assert!(correlation.take_readback_trace().is_none());
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.readback_correlation_overflows, 1);
    assert_eq!(
        snapshot.readback_correlation_high_water as usize,
        MAX_PENDING_READBACK_TRACES
    );
    assert_eq!(snapshot.readback_identity_misses, 0);
}

#[test]
fn strips_gpu_row_padding_from_rgba_readback() {
    let width = 130;
    let height = 2;
    let row_bytes = width * 4;
    let aligned_row_bytes = RenderDevice::align_copy_bytes_per_row(row_bytes as usize);
    let mut data = vec![0u8; aligned_row_bytes * height as usize];
    data[..row_bytes as usize].fill(1);
    data[aligned_row_bytes..aligned_row_bytes + row_bytes as usize].fill(2);

    let (rgba, repacked) =
        unpack_rgba_readback(data, width, height).expect("padded readback is valid");

    assert_eq!(rgba.len(), row_bytes as usize * height as usize);
    assert!(rgba[..row_bytes as usize].iter().all(|byte| *byte == 1));
    assert!(rgba[row_bytes as usize..].iter().all(|byte| *byte == 2));
    assert!(repacked);
}

#[test]
fn compacts_padded_rows_without_changing_the_frame_bytes() {
    let width = 130;
    let height = 3;
    let row_bytes = width * 4;
    let aligned_row_bytes = RenderDevice::align_copy_bytes_per_row(row_bytes as usize);
    let mut data = vec![0u8; aligned_row_bytes * height as usize];
    for row in 0..height as usize {
        let start = row * aligned_row_bytes;
        data[start..start + row_bytes as usize].fill((row + 3) as u8);
    }
    let original_capacity = data.capacity();

    let (rgba, repacked) =
        unpack_rgba_readback(data, width, height).expect("padded readback is valid");

    assert!(repacked);
    assert_eq!(rgba.capacity(), original_capacity);
    for row in 0..height as usize {
        let start = row * row_bytes as usize;
        assert!(
            rgba[start..start + row_bytes as usize]
                .iter()
                .all(|byte| *byte == (row + 3) as u8)
        );
    }
}

#[test]
fn keeps_aligned_rgba_readback_unchanged() {
    let width = 64;
    let height = 2;
    let data = vec![7u8; width as usize * height as usize * 4];

    assert_eq!(
        unpack_rgba_readback(data.clone(), width, height),
        Some((data, false))
    );
}
