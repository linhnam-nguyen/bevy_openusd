use super::*;

#[test]
fn local_requests_are_ordered_and_identifiable() {
    let mut inbox = ViewportCommandInbox::default();

    assert_eq!(inbox.send(ViewportCommand::ReloadSession), "local-1");
    assert_eq!(inbox.send(ViewportCommand::RequestSnapshot), "local-2");
    assert_eq!(inbox.pop().unwrap().request_id, "local-1");
    assert_eq!(inbox.pop().unwrap().request_id, "local-2");
}

#[test]
fn external_requests_preserve_their_correlation_id() {
    let mut inbox = ViewportCommandInbox::default();
    inbox.push(ViewportCommandEnvelope::new(
        "desktop-3",
        ViewportCommand::RequestSnapshot,
    ));

    assert_eq!(inbox.pop().unwrap().request_id, "desktop-3");
}

#[test]
fn published_events_are_available_to_the_local_reducer_and_transport() {
    let mut outbox = ViewportEventOutbox::default();
    let event = ViewportEventEnvelope::new(
        Some("local-1".into()),
        viewport_protocol::ViewportEvent::Ready {
            protocol_version: viewport_protocol::PROTOCOL_VERSION,
        },
    );

    outbox.push(event.clone());

    assert_eq!(outbox.take_published(), vec![event.clone()]);
    assert_eq!(outbox.pop(), Some(event));
}

#[test]
fn focus_queue_keeps_latest_value_behind_control_work() {
    let mut inbox = ViewportTreeCommandInbox::default();
    let heavy = SceneAnchor::active_session("/World/Heavy");
    let light = SceneAnchor::active_session("/World/Light");

    inbox.push(ViewportTreeCommand::Focus {
        request_id: "heavy-focus".into(),
        target: heavy,
        mode: FocusMode::FrameTarget,
        selection_revision: 1,
        scene_revision: 1,
        generation: 0,
    });
    inbox.push(ViewportTreeCommand::Focus {
        request_id: "light-focus".into(),
        target: light.clone(),
        mode: FocusMode::FlyToTarget,
        selection_revision: 2,
        scene_revision: 1,
        generation: 0,
    });
    inbox.push(ViewportTreeCommand::SetSubtreeVisibility {
        request_id: "visibility".into(),
        target: light,
        visible: false,
    });

    assert_eq!(inbox.pending_focus_count(), 1);
    assert!(matches!(
        inbox.pop(),
        Some(ViewportTreeCommand::SetSubtreeVisibility { .. })
    ));
    let Some(ViewportTreeCommand::Focus {
        request_id,
        mode,
        generation,
        ..
    }) = inbox.pop()
    else {
        panic!("latest focus must remain queued");
    };
    assert_eq!(request_id, "light-focus");
    assert_eq!(mode, FocusMode::FlyToTarget);
    assert_eq!(generation, 2);
    assert_eq!(inbox.pending_focus_count(), 0);
}

#[test]
fn stale_focus_is_removed_when_selection_revision_changes() {
    let mut inbox = ViewportTreeCommandInbox::default();
    inbox.push(ViewportTreeCommand::Focus {
        request_id: "heavy-focus".into(),
        target: SceneAnchor::active_session("/World/Heavy"),
        mode: FocusMode::FrameTarget,
        selection_revision: 7,
        scene_revision: 3,
        generation: 0,
    });

    inbox.cancel_focus_if_stale(8, 3);

    assert_eq!(inbox.pending_focus_count(), 0);
    assert!(inbox.pop().is_none());
}
