use super::*;

#[test]
fn overlay_plan_maps_presence_and_path_changes() {
    assert_eq!(
        overlay_plan(PresenceState::Added, ChangeFlags::empty()),
        OverlayPlan {
            current_added: true,
            ..Default::default()
        }
    );
    assert_eq!(
        overlay_plan(PresenceState::Removed, ChangeFlags::empty()),
        OverlayPlan {
            historical_removed: true,
            ..Default::default()
        }
    );
    assert_eq!(
        overlay_plan(PresenceState::Existing, ChangeFlags::PATH),
        OverlayPlan {
            current_changed: true,
            historical_moved: true,
            ..Default::default()
        }
    );
}

#[test]
fn unchanged_entities_have_no_overlay_plan() {
    assert_eq!(
        overlay_plan(PresenceState::Existing, ChangeFlags::empty()),
        OverlayPlan::default()
    );
}
