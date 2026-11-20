use super::{hosts, scene};
use crate::viewport::app::launch_options::parse_launch_options;
use scene::resolve_requested_asset;

pub(crate) fn run() {
    let launch_options = match parse_launch_options(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("usdview: {error}");
            std::process::exit(2);
        }
    };
    let (asset_path, asset_root) = resolve_requested_asset(launch_options.asset_argument.clone());
    let benchmark_scenario = launch_options
        .benchmark_scenario
        .as_deref()
        .and_then(crate::viewport::diagnostics::performance::BenchmarkScenarioId::from_code);
    let is_s8 = benchmark_scenario
        == Some(
            crate::viewport::diagnostics::performance::BenchmarkScenarioId::S8NativeNoLiveStage,
        );

    if launch_options.headless {
        hosts::headless::run(&launch_options, &asset_path, asset_root, is_s8);
    } else {
        hosts::mara::run(&launch_options, &asset_path, asset_root, is_s8);
    }
}
