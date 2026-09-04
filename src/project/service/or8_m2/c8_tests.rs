use super::*;

#[test]
fn c8_runs_the_full_projects_lifecycle_matrix_for_all_sixteen_seeds() {
    let mut selections = Vec::new();
    for seed in rng::M2_SEEDS {
        matrix::clean_previous_attempts(seed).unwrap_or_else(|error| {
            panic!("C8 seed {seed:#018X} could not clean previous attempts: {error}");
        });
        let selection = match matrix::run_seed(seed, 1) {
            Ok(selection) => selection,
            Err(first_error) => {
                let mut rerun_errors = vec![first_error];
                for attempt in 2..=4 {
                    if let Err(error) = matrix::run_seed(seed, attempt) {
                        rerun_errors.push(error);
                    }
                }
                panic!(
                    "C8 seed {seed:#018X} failed; exactly three reruns completed: {rerun_errors:?}"
                );
            }
        };
        selections.push(selection);
    }
    let depths = selections
        .iter()
        .flat_map(|selection| [selection.source_depth, selection.target_depth])
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        depths.len() > 1,
        "the 16-seed C8 corpus must exercise more than one hierarchy depth: {selections:?}"
    );
}
