//! Performance sample data types and versioned benchmark identity.

use serde::{Deserialize, Serialize};

/// Version of the benchmark sample and report schema.
pub const SCHEMA_VERSION: u32 = 1;

/// Immutable environment and run metadata for benchmark reproducibility.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkIdentity {
    pub schema_version: u32,
    pub checkpoint_id: String,
    pub git_sha: String,
    pub glacial_sha: String,
    pub scenario_code: Option<String>,
    pub scene_label: String,
    pub scene_path: String,
    pub build_profile: String,
    pub os: String,
    pub backend: String,
    pub gpu_adapter: String,
    pub width: u32,
    pub height: u32,
    pub requested_fps: f64,
}

impl BenchmarkIdentity {
    pub fn new(
        checkpoint_id: &str,
        scene_label: &str,
        scenario_code: Option<String>,
        gpu_adapter: String,
        width: u32,
        height: u32,
        requested_fps: f64,
    ) -> Self {
        let git_sha = option_env!("USDHUB_GIT_SHA")
            .unwrap_or("unknown")
            .to_string();
        let glacial_sha = option_env!("USDHUB_GLACIAL_SHA")
            .unwrap_or("unknown")
            .to_string();
        let build_profile = if cfg!(debug_assertions) {
            "debug".to_string()
        } else {
            "release".to_string()
        };
        let os = std::env::consts::OS.to_string();
        let backend = if cfg!(target_os = "macos") {
            "metal".to_string()
        } else {
            "vulkan".to_string()
        };

        Self {
            schema_version: SCHEMA_VERSION,
            checkpoint_id: checkpoint_id.to_string(),
            git_sha,
            glacial_sha,
            scenario_code,
            scene_label: scene_label.to_string(),
            scene_path: scene_label.to_string(),
            build_profile,
            os,
            backend,
            gpu_adapter,
            width,
            height,
            requested_fps,
        }
    }
}

/// Active renderer configuration during benchmark execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderConfiguration {
    pub grid: bool,
    pub shadows: bool,
    pub edges: bool,
    pub render_mode: RenderMode,
    pub material_overrides: bool,
}

/// Rendering visual mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderMode {
    Shaded,
    Flat,
    Wireframe,
}

impl Default for RenderConfiguration {
    fn default() -> Self {
        Self {
            grid: true,
            shadows: true,
            edges: false,
            render_mode: RenderMode::Shaded,
            material_overrides: true,
        }
    }
}

/// Sample recording metrics for a single rendered frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameSample {
    pub frame_index: u64,
    pub cpu_duration_ms: f64,
    pub wall_interval_ms: Option<f64>,
    pub gpu_duration_ms: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_identity_records_git_and_glacial_sha() {
        let identity = BenchmarkIdentity::new(
            "M1-C1",
            "hummingbird.usdz",
            Some("S1".into()),
            "Apple M4".into(),
            1920,
            1080,
            60.0,
        );
        assert_eq!(identity.checkpoint_id, "M1-C1");
        assert_eq!(identity.scene_label, "hummingbird.usdz");
        assert_eq!(identity.scenario_code, Some("S1".into()));
        assert_eq!(identity.gpu_adapter, "Apple M4");
        assert!(!identity.git_sha.is_empty());
        assert!(!identity.glacial_sha.is_empty());
        assert_eq!(identity.width, 1920);
        assert_eq!(identity.height, 1080);
        assert_eq!(identity.requested_fps, 60.0);
    }

    #[test]
    fn render_configuration_defaults_and_round_trip() {
        let config = RenderConfiguration::default();
        assert!(config.grid);
        assert!(config.shadows);
        assert!(!config.edges);
        assert_eq!(config.render_mode, RenderMode::Shaded);

        let json = serde_json::to_string(&config).expect("must serialize");
        let deserialized: RenderConfiguration =
            serde_json::from_str(&json).expect("must deserialize");
        assert_eq!(config, deserialized);
    }
}
