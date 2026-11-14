//! Pinned deterministic decisions for the OR8 M2 seed corpus.

/// The frozen sixteen-seed M2 corpus from the OR8 implementation plan.
pub(super) const M2_SEEDS: [u64; 16] = [
    0x4F52380000000001,
    0x4F52380000000002,
    0x4F52380000000003,
    0x4F52380000000004,
    0x4F52380000000005,
    0x4F52380000000006,
    0x4F52380000000007,
    0x4F52380000000008,
    0x4F52380000000009,
    0x4F5238000000000A,
    0x4F5238000000000B,
    0x4F5238000000000C,
    0x4F5238000000000D,
    0x4F5238000000000E,
    0x4F5238000000000F,
    0x4F52380000000010,
];

/// A small, explicitly pinned SplitMix64 stream.
///
/// M2 does not need cryptographic randomness. This generator is kept local so
/// the scenario has no ambient-thread RNG, no hidden global state, and no
/// dependency-version drift. Each decision is recorded by the caller.
#[derive(Clone, Debug)]
pub(super) struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    pub(super) fn seeded(seed: u64) -> Self {
        Self { state: seed }
    }

    pub(super) fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D049BB133111EB);
        value ^ (value >> 31)
    }

    pub(super) fn choose_index(&mut self, length: usize) -> usize {
        assert!(length > 0, "deterministic choice requires a non-empty set");
        (self.next_u64() as usize) % length
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_is_exactly_sixteen_sequential_seed_values() {
        assert_eq!(M2_SEEDS.len(), 16);
        for (index, seed) in M2_SEEDS.iter().enumerate() {
            assert_eq!(*seed, 0x4F52380000000001 + index as u64);
        }
    }

    #[test]
    fn stream_is_reproducible_without_thread_rng() {
        let mut first = DeterministicRng::seeded(M2_SEEDS[0]);
        let mut second = DeterministicRng::seeded(M2_SEEDS[0]);
        assert_eq!(
            (0..32).map(|_| first.next_u64()).collect::<Vec<_>>(),
            (0..32).map(|_| second.next_u64()).collect::<Vec<_>>()
        );
    }
}
