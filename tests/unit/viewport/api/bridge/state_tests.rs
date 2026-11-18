use super::*;

#[test]
fn save_state_is_separate_from_undo_history() {
    let mut histories = EditorHistories::default();
    histories.record(EditorHistoryDomain::Authoring);
    assert!(histories.state().is_dirty);
    assert!(histories.state().can_undo);

    histories.mark_saved();

    assert!(!histories.state().is_dirty);
    assert!(histories.state().can_undo);
}
