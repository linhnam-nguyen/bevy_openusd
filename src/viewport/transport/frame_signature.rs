//! Bounded, opt-in render samples for the animation diagnostic.

use bevy::prelude::Resource;

pub(crate) const SIGNATURE_SIDE: usize = 64;
pub(crate) const SIGNATURE_SAMPLE_COUNT: usize = SIGNATURE_SIDE * SIGNATURE_SIDE;
const MAX_CAPTURES: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FrameLumaSample {
    pub(crate) hash: u64,
    pub(crate) mean_luma: f64,
    luma: [u8; SIGNATURE_SAMPLE_COUNT],
}

impl FrameLumaSample {
    pub(crate) fn mad(&self, other: &Self) -> f64 {
        let sum: u64 = self
            .luma
            .iter()
            .zip(other.luma.iter())
            .map(|(left, right)| u64::from(left.abs_diff(*right)))
            .sum();
        sum as f64 / SIGNATURE_SAMPLE_COUNT as f64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameSampleId {
    T0,
    T1,
    T0RoundTrip,
    StaticT0,
    StaticT1,
}

#[derive(Clone, Debug)]
pub(crate) struct CapturedFrameSignature {
    pub(crate) id: FrameSampleId,
    pub(crate) sequence: u64,
    pub(crate) sample: FrameLumaSample,
}

#[derive(Clone, Copy, Debug)]
struct PendingCapture {
    id: FrameSampleId,
    min_sequence: u64,
}

#[derive(Resource, Default)]
pub(crate) struct FrameSignatureDiagnostic {
    sampler: FrameSignatureSampler,
    pending: Option<PendingCapture>,
    captures: Vec<CapturedFrameSignature>,
    previous_sample: Option<FrameLumaSample>,
    last_adjacent_mad_luma: Option<f64>,
}

impl FrameSignatureDiagnostic {
    pub(crate) fn arm(&mut self, id: FrameSampleId, min_sequence: u64) {
        if self.pending.is_some()
            || self.capture(id).is_some()
            || self.captures.len() >= MAX_CAPTURES
        {
            return;
        }
        self.pending = Some(PendingCapture { id, min_sequence });
    }

    pub(crate) fn sample(
        &mut self,
        sequence: u64,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<FrameLumaSample> {
        let pending = self
            .pending
            .filter(|pending| sequence >= pending.min_sequence)?;
        let sample = self.sampler.sample(rgba, width, height)?;
        self.last_adjacent_mad_luma = self
            .previous_sample
            .as_ref()
            .map(|previous| sample.mad(previous));
        self.previous_sample = Some(sample.clone());
        self.pending = None;
        self.captures.push(CapturedFrameSignature {
            id: pending.id,
            sequence,
            sample: sample.clone(),
        });
        Some(sample)
    }

    pub(crate) fn capture(&self, id: FrameSampleId) -> Option<&CapturedFrameSignature> {
        self.captures.iter().find(|capture| capture.id == id)
    }

    pub(crate) fn last_adjacent_mad_luma(&self) -> Option<f64> {
        self.last_adjacent_mad_luma
    }

    pub(crate) fn captures(&self) -> &[CapturedFrameSignature] {
        &self.captures
    }
}

#[derive(Default)]
pub(crate) struct FrameSignatureSampler;

impl FrameSignatureSampler {
    pub(crate) fn sample(&self, rgba: &[u8], width: u32, height: u32) -> Option<FrameLumaSample> {
        if width == 0 || height == 0 {
            return None;
        }
        let width_usize = usize::try_from(width).ok()?;
        let height_usize = usize::try_from(height).ok()?;
        let row_stride = width_usize.checked_mul(4)?;
        let expected_bytes = row_stride.checked_mul(height_usize)?;
        if rgba.len() != expected_bytes {
            return None;
        }

        let mut hash = 0xcbf29ce484222325;
        for byte in width.to_le_bytes().into_iter().chain(height.to_le_bytes()) {
            fnv1a_update(&mut hash, byte);
        }

        let mut luma = [0; SIGNATURE_SAMPLE_COUNT];
        let mut luma_sum = 0_u64;
        for sample_y in 0..SIGNATURE_SIDE {
            let source_y = sample_y.checked_mul(height_usize)? / SIGNATURE_SIDE;
            let row_start = source_y.checked_mul(row_stride)?;
            for sample_x in 0..SIGNATURE_SIDE {
                let source_x = sample_x.checked_mul(width_usize)? / SIGNATURE_SIDE;
                let pixel_start = row_start.checked_add(source_x.checked_mul(4)?)?;
                let value = luma_value(
                    rgba[pixel_start],
                    rgba[pixel_start + 1],
                    rgba[pixel_start + 2],
                );
                let sample_index = sample_y * SIGNATURE_SIDE + sample_x;
                luma[sample_index] = value;
                luma_sum += u64::from(value);
                fnv1a_update(&mut hash, value);
            }
        }

        Some(FrameLumaSample {
            hash,
            mean_luma: luma_sum as f64 / SIGNATURE_SAMPLE_COUNT as f64,
            luma,
        })
    }
}

fn luma_value(red: u8, green: u8, blue: u8) -> u8 {
    ((u32::from(red) * 77 + u32::from(green) * 150 + u32::from(blue) * 29) >> 8) as u8
}

fn fnv1a_update(hash: &mut u64, byte: u8) {
    *hash ^= u64::from(byte);
    *hash = hash.wrapping_mul(0x100000001b3);
}

#[cfg(test)]
mod tests {
    use super::{
        FrameLumaSample, FrameSampleId, FrameSignatureDiagnostic, FrameSignatureSampler,
        SIGNATURE_SAMPLE_COUNT,
    };

    fn solid_frame(width: usize, height: usize, value: u8) -> Vec<u8> {
        (0..width * height)
            .flat_map(|_| [value, value, value, u8::MAX])
            .collect()
    }

    #[test]
    fn identical_rgba_has_identical_hash_and_zero_mad() {
        let sampler = FrameSignatureSampler;
        let frame = solid_frame(128, 96, 32);
        let first = sampler.sample(&frame, 128, 96).expect("first sample");
        let second = sampler.sample(&frame, 128, 96).expect("second sample");
        assert_eq!(first.hash, second.hash);
        assert_eq!(first.mad(&second), 0.0);
    }

    #[test]
    fn different_rgb_has_different_hash_and_positive_mad() {
        let sampler = FrameSignatureSampler;
        let first = sampler
            .sample(&solid_frame(128, 96, 32), 128, 96)
            .expect("first sample");
        let second = sampler
            .sample(&solid_frame(128, 96, 96), 128, 96)
            .expect("second sample");
        assert_ne!(first.hash, second.hash);
        assert!(first.mad(&second) > 0.0);
    }

    #[test]
    fn alpha_only_difference_is_ignored() {
        let sampler = FrameSignatureSampler;
        let mut first = solid_frame(128, 96, 32);
        let mut second = first.clone();
        for pixel in first.chunks_exact_mut(4) {
            pixel[3] = 0;
        }
        for pixel in second.chunks_exact_mut(4) {
            pixel[3] = u8::MAX;
        }
        let first = sampler.sample(&first, 128, 96).expect("first sample");
        let second = sampler.sample(&second, 128, 96).expect("second sample");
        assert_eq!(first.hash, second.hash);
        assert_eq!(first.mad(&second), 0.0);
    }

    #[test]
    fn mean_luma_and_sample_count_are_deterministic() {
        let sampler = FrameSignatureSampler;
        let sample = sampler
            .sample(&solid_frame(128, 96, 32), 128, 96)
            .expect("valid sample");
        assert_eq!(SIGNATURE_SAMPLE_COUNT, 4096);
        assert_eq!(sample.mean_luma, 32.0);
        assert_eq!(sample.luma.len(), SIGNATURE_SAMPLE_COUNT);
    }

    #[test]
    fn malformed_rgba_is_rejected() {
        let sampler = FrameSignatureSampler;
        assert!(sampler.sample(&[0; 3], 1, 1).is_none());
        assert!(sampler.sample(&[], 0, 1).is_none());
    }

    #[test]
    fn named_capture_waits_for_its_sequence_floor() {
        let mut diagnostic = FrameSignatureDiagnostic::default();
        diagnostic.arm(FrameSampleId::T0, 4);
        let frame = solid_frame(64, 64, 10);
        assert!(diagnostic.sample(3, &frame, 64, 64).is_none());
        assert!(diagnostic.sample(4, &frame, 64, 64).is_some());
        let capture = diagnostic.capture(FrameSampleId::T0).expect("T0 capture");
        assert_eq!(capture.sequence, 4);
        assert_eq!(capture.sample.mean_luma, 10.0);
    }

    #[test]
    fn named_capture_ids_are_not_overwritten() {
        let mut diagnostic = FrameSignatureDiagnostic::default();
        let frame = solid_frame(64, 64, 10);
        diagnostic.arm(FrameSampleId::T0, 1);
        diagnostic.sample(1, &frame, 64, 64).expect("first capture");
        diagnostic.arm(FrameSampleId::T0, 2);
        diagnostic.sample(2, &solid_frame(64, 64, 20), 64, 64);
        assert_eq!(diagnostic.captures().len(), 1);
        assert_eq!(diagnostic.capture(FrameSampleId::T0).unwrap().sequence, 1);
    }

    #[test]
    fn diagnostic_sample_preserves_adjacent_mad_for_supporting_metrics() {
        let mut diagnostic = FrameSignatureDiagnostic::default();
        let first = solid_frame(64, 64, 10);
        let second = solid_frame(64, 64, 20);
        diagnostic.arm(FrameSampleId::T0, 1);
        diagnostic.sample(1, &first, 64, 64).expect("first capture");
        diagnostic.arm(FrameSampleId::T1, 2);
        diagnostic
            .sample(2, &second, 64, 64)
            .expect("second capture");
        assert!(diagnostic.last_adjacent_mad_luma().unwrap() > 0.0);
    }

    #[test]
    fn luma_sample_is_cloneable_without_exposing_pixels() {
        let sample = FrameLumaSample {
            hash: 1,
            mean_luma: 2.0,
            luma: [3; SIGNATURE_SAMPLE_COUNT],
        };
        assert_eq!(sample.clone(), sample);
    }
}
