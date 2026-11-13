use super::*;

#[test]
fn all_24_scenarios_are_categorized_and_code_stable() {
    let all = BenchmarkScenarioId::all();
    assert_eq!(all.len(), 24);

    let mut native_count = 0;
    let mut webrtc_count = 0;
    let mut isolation_count = 0;

    for (idx, scenario) in all.iter().enumerate() {
        let code = scenario.code();
        assert_eq!(code, format!("S{}", idx + 1));

        match scenario.category() {
            ScenarioCategory::NativeSteadyState => native_count += 1,
            ScenarioCategory::WebRtcRemotePath => webrtc_count += 1,
            ScenarioCategory::RenderDataPlaneIsolation => isolation_count += 1,
        }

        let def = ScenarioProbeDefinition::for_scenario(*scenario);
        assert_eq!(def.code, code);
    }

    assert_eq!(native_count, 10);
    assert_eq!(webrtc_count, 8);
    assert_eq!(isolation_count, 6);
}

#[test]
fn scenario_probe_serialization_round_trip() {
    for scenario in BenchmarkScenarioId::all() {
        let def = ScenarioProbeDefinition::for_scenario(*scenario);
        let json = serde_json::to_string(&def).expect("must serialize");
        let deserialized: ScenarioProbeDefinition =
            serde_json::from_str(&json).expect("must deserialize");
        assert_eq!(def.id, deserialized.id);
        assert_eq!(def.code, deserialized.code);
        assert_eq!(def.category, deserialized.category);
    }
}

#[test]
fn steady_state_expectation_detects_mismatch() {
    let expected = SteadyStateExpectations::default();
    assert_eq!(expected.grid_structural_rebuilds, 0);
    assert_eq!(expected.semantic_snapshot_clones, 0);
    assert_eq!(expected.recovery_checkpoints, 0);
    assert_eq!(expected.sync_db_auth_waits_in_bevy, 0);

    let mut observed = expected.clone();
    observed.grid_structural_rebuilds = 1;

    assert_ne!(expected, observed);
}
