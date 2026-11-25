use super::*;
use crate::viewport::navigation::input::HostUiInputCapture;

#[test]
fn headless_host_stays_windowless_with_shared_ui_capture_inactive() {
    let mut app = App::new();
    app.init_resource::<Assets<Image>>()
        .init_resource::<HostUiInputCapture>()
        .add_plugins(HeadlessRenderPlugin {
            width: 640,
            height: 360,
        });

    let capture = app.world().resource::<HostUiInputCapture>();
    assert!(!capture.wants_pointer);
    assert!(!capture.wants_keyboard);
    let target = app.world().resource::<OffscreenTarget>();
    assert_eq!((target.width, target.height), (640, 360));
}
