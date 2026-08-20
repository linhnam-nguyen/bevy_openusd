use super::*;

#[test]
fn s2_driver_disables_grid_on_startup() {
    let mut app = App::new();
    app.insert_resource(ActiveScenarioDriver {
        scenario_id: Some(BenchmarkScenarioId::S2NativeHummingbirdGridOffPaused),
        frame_counter: 0,
        action_executions: 0,
    });
    app.insert_resource(GroundGrid {
        visible: true,
        ..Default::default()
    });
    app.insert_resource(DisplayToggles {
        show_world_grid: true,
        ..Default::default()
    });
    app.add_systems(Startup, setup_scenario_driver_system);
    app.update();

    assert!(!app.world().resource::<GroundGrid>().visible);
    assert!(!app.world().resource::<DisplayToggles>().show_world_grid);
}

#[test]
fn s4_driver_toggles_grid_visibility() {
    let mut app = App::new();
    app.insert_resource(ActiveScenarioDriver {
        scenario_id: Some(BenchmarkScenarioId::S4NativeGridVisibilityToggle),
        frame_counter: 14,
        action_executions: 0,
    });
    app.insert_resource(BenchmarkRunState {
        scene_ready: true,
        client_ready: false,
        measurement_started: false,
        measurement_idle_signaled: false,
        warmup_frames_remaining: 0,
        target_frames_remaining: 120,
        samples: vec![],
        is_completed: false,
    });
    app.insert_resource(GroundGrid {
        visible: true,
        ..Default::default()
    });
    app.insert_resource(DisplayToggles {
        show_world_grid: true,
        ..Default::default()
    });
    app.add_systems(Update, scenario_action_driver_system);
    app.update();

    assert!(!app.world().resource::<DisplayToggles>().show_world_grid);
    assert_eq!(
        app.world()
            .resource::<ActiveScenarioDriver>()
            .action_executions,
        1
    );
}
