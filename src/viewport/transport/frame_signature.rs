//! Fixed-size render signatures for opt-in animation diagnostics.

use bevy::prelude::Resource;

pub(crate) const SIGNATURE_SIDE: usize = 64;
pub(crate) const SIGNATURE_SAMPLE_COUNT: usize = SIGNATURE_SIDE * SIGNATURE_SIDE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameSignature {
    pub(crate) hash: u64,
    pub(crate) mad_luma_sum: Option<u64>,
}

pub(crate) struct FrameSignatureSampler {
    previous_luma: [u8; SIGNATURE_SAMPLE_COUNT],
    current_luma: [u8; SIGNATURE_SAMPLE_COUNT],
    has_previous: bool,
}

#[derive(Resource)]
pub(crate) struct FrameSignatureDiagnostic {
    sampler: FrameSignatureSampler,
}

impl Default for FrameSignatureDiagnostic {
    fn default() -> Self {
        Self {
            sampler: FrameSignatureSampler::default(),
        }
    }
}

impl FrameSignatureDiagnostic {
    pub(crate) fn sample(
        &mut self,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<FrameSignature> {
        self.sampler.sample(rgba, width, height)
    }
}

impl Default for FrameSignatureSampler {
    fn default() -> Self {
        Self {
            previous_luma: [0; SIGNATURE_SAMPLE_COUNT],
            current_luma: [0; SIGNATURE_SAMPLE_COUNT],
            has_previous: false,
        }
    }
}

impl FrameSignatureSampler {
    pub(crate) fn sample(
        &mut self,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<FrameSignature> {
        if width == 0 || height == 0 {
            return None;
        }
        let width_u32 = width;
        let height_u32 = height;
        let width = usize::try_from(width_u32).ok()?;
        let height = usize::try_from(height_u32).ok()?;
        let row_stride = width.checked_mul(4)?;
        let expected_bytes = row_stride.checked_mul(height)?;
        if rgba.len() != expected_bytes {
            return None;
        }

        let mut hash = 0xcbf29ce484222325;
        for byte in width_u32
            .to_le_bytes()
            .into_iter()
            .chain(height_u32.to_le_bytes())
        {
            fnv1a_update(&mut hash, byte);
        }

        for sample_y in 0..SIGNATURE_SIDE {
            let source_y = sample_y.checked_mul(height)? / SIGNATURE_SIDE;
            let row_start = source_y.checked_mul(row_stride)?;
            for sample_x in 0..SIGNATURE_SIDE {
                let source_x = sample_x.checked_mul(width)? / SIGNATURE_SIDE;
                let pixel_start = row_start.checked_add(source_x.checked_mul(4)?)?;
                let red = rgba[pixel_start];
                let green = rgba[pixel_start + 1];
                let blue = rgba[pixel_start + 2];
                let luma = luma(red, green, blue);
                let sample_index = sample_y * SIGNATURE_SIDE + sample_x;
                self.current_luma[sample_index] = luma;
                fnv1a_update(&mut hash, luma);
            }
        }

        let mad_luma_sum = self.has_previous.then(|| {
            self.current_luma
                .iter()
                .zip(self.previous_luma.iter())
                .map(|(current, previous)| u64::from(current.abs_diff(*previous)))
                .sum()
        });
        std::mem::swap(&mut self.previous_luma, &mut self.current_luma);
        self.has_previous = true;

        Some(FrameSignature { hash, mad_luma_sum })
    }
}

fn luma(red: u8, green: u8, blue: u8) -> u8 {
    ((u16::from(red) * 54 + u16::from(green) * 183 + u16::from(blue) * 19 + 128) / 256) as u8
}

fn fnv1a_update(hash: &mut u64, byte: u8) {
    *hash ^= u64::from(byte);
    *hash = hash.wrapping_mul(0x100000001b3);
}

#[cfg(test)]
mod tests {
    use super::{FrameSignatureSampler, SIGNATURE_SAMPLE_COUNT};

    fn solid_frame(width: usize, height: usize, value: u8) -> Vec<u8> {
        vec![value; width * height * 4]
    }

    #[test]
    fn first_sample_has_stable_hash_without_mad() {
        let mut sampler = FrameSignatureSampler::default();
        let frame = solid_frame(128, 96, 32);

        let signature = sampler
            .sample(&frame, 128, 96)
            .expect("valid RGBA frame must produce a signature");

        assert_ne!(signature.hash, 0);
        assert_eq!(signature.mad_luma_sum, None);
    }

    #[test]
    fn identical_frames_have_zero_mad() {
        let mut sampler = FrameSignatureSampler::default();
        let frame = solid_frame(128, 96, 32);

        sampler.sample(&frame, 128, 96).expect("first sample");
        let signature = sampler.sample(&frame, 128, 96).expect("second sample");

        assert_eq!(signature.mad_luma_sum, Some(0));
    }

    #[test]
    fn changed_luma_produces_nonzero_mad_and_hash() {
        let mut sampler = FrameSignatureSampler::default();
        let first = solid_frame(128, 96, 32);
        let second = solid_frame(128, 96, 96);

        let first_signature = sampler.sample(&first, 128, 96).expect("first sample");
        let second_signature = sampler.sample(&second, 128, 96).expect("second sample");

        assert_ne!(first_signature.hash, second_signature.hash);
        assert!(
            second_signature
                .mad_luma_sum
                .expect("second sample has a predecessor")
                > 0
        );
    }

    #[test]
    fn sample_count_is_fixed_and_bounded() {
        assert_eq!(SIGNATURE_SAMPLE_COUNT, 4096);
    }

    #[test]
    fn malformed_rgba_buffer_is_rejected() {
        let mut sampler = FrameSignatureSampler::default();

        assert_eq!(sampler.sample(&[0; 3], 1, 1), None);
        assert_eq!(sampler.sample(&[], 0, 1), None);
    }

    #[test]
    fn alpha_only_changes_do_not_change_luma_signature() {
        let mut sampler = FrameSignatureSampler::default();
        let mut first = solid_frame(128, 96, 32);
        let mut second = first.clone();
        for alpha in first.chunks_exact_mut(4).map(|pixel| &mut pixel[3]) {
            *alpha = 0;
        }
        for alpha in second.chunks_exact_mut(4).map(|pixel| &mut pixel[3]) {
            *alpha = 255;
        }

        let first_signature = sampler.sample(&first, 128, 96).expect("first sample");
        let second_signature = sampler.sample(&second, 128, 96).expect("second sample");

        assert_eq!(first_signature.hash, second_signature.hash);
        assert_eq!(second_signature.mad_luma_sum, Some(0));
    }
}
