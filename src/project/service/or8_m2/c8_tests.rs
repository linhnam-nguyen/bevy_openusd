use super::*;

#[test]
fn c8_runs_the_full_projects_lifecycle_matrix_for_all_sixteen_seeds() {
    for seed in rng::M2_SEEDS {
        matrix::clean_previous_attempts(seed).unwrap_or_else(|error| {
            panic!("C8 seed {seed:#018X} could not clean previous attempts: {error}");
        });
        let first = matrix::run_seed(seed, 1);
        if let Err(first_error) = first {
            let mut rerun_errors = vec![first_error];
            for attempt in 2..=4 {
                if let Err(error) = matrix::run_seed(seed, attempt) {
                    rerun_errors.push(error);
                }
            }
            panic!("C8 seed {seed:#018X} failed; exactly three reruns completed: {rerun_errors:?}");
        }
    }
}
