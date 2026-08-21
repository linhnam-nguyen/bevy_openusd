use serde::{Deserialize, Serialize};

use super::aggregate::RendererCadenceSummary;
use super::runner::BenchmarkLaunchConfig;
use super::sample::RenderConfiguration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RendererMatrixCaseReport {
    pub requested: RenderConfiguration,
    pub effective: RenderConfiguration,
    pub accepted: bool,
    pub configuration_matches: bool,
    pub measured_frames: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RendererMatrixCadenceReport {
    pub requested_fps: u32,
    pub summary: RendererCadenceSummary,
    pub measured_frames: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RendererMatrixReport {
    pub schema_version: u32,
    pub release_profile: bool,
    pub cases: Vec<RendererMatrixCaseReport>,
    pub cadence_samples: Vec<RendererMatrixCadenceReport>,
    pub passed: bool,
}

pub fn finalize_matrix_report(
    config: &BenchmarkLaunchConfig,
    cases: &[RendererMatrixCaseReport],
    cadence_samples: &[RendererMatrixCadenceReport],
) {
    let passed = cases.len() == 16
        && cadence_samples.len() == 3
        && cases
            .iter()
            .all(|case| case.accepted && case.configuration_matches)
        && cadence_samples.iter().all(|sample| {
            sample.summary.effective_renderer_target_fps == Some(sample.requested_fps)
                && sample.summary.actual_rendered_fps.is_some()
        });
    let report = RendererMatrixReport {
        schema_version: 1,
        release_profile: !cfg!(debug_assertions),
        cases: cases.to_vec(),
        cadence_samples: cadence_samples.to_vec(),
        passed,
    };
    if let Some(path) = config.output_path.as_ref() {
        write_report(path, &report);
    }
    if !passed {
        eprintln!("M3-C6 renderer matrix benchmark failed: configuration mismatch");
        std::process::exit(1);
    }
    std::process::exit(0);
}

fn write_report(path: &std::path::Path, report: &RendererMatrixReport) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(report) {
        let _ = std::fs::write(path, json);
    }
}
