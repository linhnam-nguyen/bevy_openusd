use bevy::prelude::Resource;

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
        }
    }
}

impl UsdStageTime {
    /// Returns the current playback position in USD time-code units.
    pub fn current_time_code(&self) -> f64 {
        self.start_time_code + self.seconds * self.time_codes_per_second
    }

    /// Returns the authored playback range expressed in seconds.
    pub fn duration_seconds(&self) -> f64 {
        (self.end_time_code - self.start_time_code).max(0.0) / self.time_codes_per_second
    }
}
