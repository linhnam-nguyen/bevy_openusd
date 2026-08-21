use bevy::prelude::*;

use super::runner::{BenchmarkLaunchConfig, BenchmarkRunState, benchmark_stepper_system};

/// Plugin registering benchmark resources and the selected stepper system.
pub struct BenchmarkRunnerPlugin {
    pub config: BenchmarkLaunchConfig,
}

impl Plugin for BenchmarkRunnerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.config.clone());
        if self.config.renderer_matrix {
            app.insert_resource(super::matrix::RendererMatrixRun::new())
                .add_systems(Last, super::matrix::renderer_matrix_stepper_system);
        } else {
            app.insert_resource(BenchmarkRunState::new(
                self.config.warmup_frames,
                self.config.target_frames,
            ))
            .add_systems(Last, benchmark_stepper_system);
        }
    }
}
