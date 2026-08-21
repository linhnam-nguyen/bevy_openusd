use super::*;

#[test]
fn matrix_contains_all_sixteen_renderer_configurations() {
    let configurations: Vec<_> = (0..16).map(matrix_configuration).collect();
    assert_eq!(configurations.len(), 16);
    assert_eq!(
        configurations.iter().filter(|config| config.grid).count(),
        8
    );
    assert_eq!(
        configurations
            .iter()
            .filter(|config| config.shadows)
            .count(),
        8
    );
    assert_eq!(
        configurations.iter().filter(|config| config.edges).count(),
        8
    );
    assert_eq!(
        configurations
            .iter()
            .filter(|config| config.render_mode == RenderMode::Wireframe)
            .count(),
        8
    );
}

#[test]
fn cadence_samples_are_the_required_authority_targets() {
    assert_eq!(MATRIX_FPS_SAMPLES, [30, 60, 120]);
}
