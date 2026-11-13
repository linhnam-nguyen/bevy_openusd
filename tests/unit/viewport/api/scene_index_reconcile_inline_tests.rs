use super::*;

#[test]
fn dense_member_removal_repairs_the_swapped_position() {
    let a = Entity::from_bits(1);
    let b = Entity::from_bits(2);
    let c = Entity::from_bits(3);
    let mut state = SceneIndexReconcileState::default();
    assert!(state.admit(a));
    assert!(state.admit(b));
    assert!(state.admit(c));

    assert!(state.remove(a));
    assert!(!state.contains(a));
    assert!(state.contains(b));
    assert!(state.contains(c));
    assert_eq!(state.members.len(), 2);
    for (index, entity) in state.members.iter().copied().enumerate() {
        assert_eq!(state.positions.get(&entity), Some(&index));
    }
}
