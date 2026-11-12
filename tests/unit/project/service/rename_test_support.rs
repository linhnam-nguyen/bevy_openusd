use std::cell::Cell;

thread_local! {
    static FAIL_AFTER_PLACEMENT: Cell<Option<usize>> = const { Cell::new(None) };
}

pub(super) fn set_test_failure_after_placement(index: usize) {
    FAIL_AFTER_PLACEMENT.with(|failure| failure.set(Some(index)));
}

pub(super) fn clear_test_failure_after_placement() {
    FAIL_AFTER_PLACEMENT.with(|failure| failure.set(None));
}

pub(super) fn should_fail_after_placement(index: usize) -> bool {
    FAIL_AFTER_PLACEMENT.with(|failure| failure.get() == Some(index))
}
