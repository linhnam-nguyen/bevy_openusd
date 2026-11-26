use bevy::prelude::Resource;

/// The authoritative clock distinguishes a qualified static stage from a domain
/// that could not be established. Neither is playable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackDomainReadiness {
    Unavailable,
    NoPlayableAnimation,
    Ready,
}

/// Animation playback clock. Time is held in seconds and translated into USD
/// time codes by the evaluator.
#[derive(Resource, Debug, Clone, Copy)]
pub struct UsdStageTime {
    pub seconds: f64,
    pub playing: bool,
    pub start_time_code: f64,
    pub end_time_code: f64,
    pub time_codes_per_second: f64,
    /// Prevents a loaded stage from overwriting a user scrub every frame.
    pub initialized: bool,
    stage_identity: Option<(u64, u64)>,
    observed_stage_range: Option<(u64, u64, u64, bool, u64)>,
    pub domain_readiness: PlaybackDomainReadiness,
}

impl Default for UsdStageTime {
    fn default() -> Self {
        Self {
            seconds: 0.0,
            playing: false,
            start_time_code: 0.0,
            end_time_code: 1.0,
            time_codes_per_second: 24.0,
            initialized: false,
            stage_identity: None,
            observed_stage_range: None,
            domain_readiness: PlaybackDomainReadiness::Unavailable,
        }
    }
}

impl UsdStageTime {
    pub(crate) fn stage_identity(&self) -> Option<(u64, u64)> {
        self.stage_identity
    }

    pub(crate) fn observed_stage_range(&self) -> Option<(u64, u64, u64, bool, u64)> {
        self.observed_stage_range
    }

    pub(crate) fn set_observed_stage_range(&mut self, range: (u64, u64, u64, bool, u64)) {
        self.observed_stage_range = Some(range);
    }

    pub(crate) fn reset_for_stage(&mut self, stage_identity: Option<(u64, u64)>) {
        *self = Self {
            stage_identity,
            ..Self::default()
        };
    }

    /// Returns the current playback position in USD time-code units.
    pub fn current_time_code(&self) -> f64 {
        self.start_time_code + self.seconds * self.time_codes_per_second
    }

    /// Returns the authored playback range expressed in seconds.
    pub fn duration_seconds(&self) -> f64 {
        (self.end_time_code - self.start_time_code).max(0.0) / self.time_codes_per_second
    }
}
