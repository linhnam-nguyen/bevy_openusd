# UP-C06 S04: event-ordered native pointer input (F-C06-I5-01)

## Scope and target

This change corrects event attribution in the Bevy `PrimaryWindow` input adapter. The shipped `usdview` CLI selects either the Mara host or the headless host. Those hosts disable Bevy Winit and use separate input adapters.

## Root cause

The original `apply_local_navigation_input` read `Messages<MouseButtonInput>` and sampled `Window::cursor_position()` and `ButtonInput<KeyCode>` only once per Update. A batch containing press/release at A, then press/release at B, consequently gave both clicks the same final cursor coordinate and final modifier state. Button-edge **order** survived, but coordinates and press-time modifiers did not.

## Corrected contract

- Consume Bevy 0.20's unified `bevy::window::WindowEvent` stream with a persistent `MessageCursor<WindowEvent>`. Bevy documents it as preserving OS order across cursor, keyboard and mouse input events.
- Track `CursorMoved` in the window's logical pixel coordinate system; preserve the latest event-reported position across Updates. On every primary `MouseButtonInput`, use the position known **at that event**. Do not substitute a later `Window::cursor_position()`.
- Track left/right Shift/Ctrl/Alt/Super key transitions in the same stream and sample them per button edge, including Ctrl/Command box selection. Frame-end `ButtonInput<KeyCode>` remains for camera navigation only: it must not override press-time modifiers because input-system scheduling can update it before or after this adapter.
- Maintain observed focus across updates so events preceding a focus gain cannot inherit the final window focus. Clear trusted pointer and gesture state on cursor exit, focus loss, viewport resize or scale change, and primary-window replacement. A missing event stream or an untrusted position invalidates the gesture.
- While viewer interaction is locked, track ordered focus and modifier transitions and suppress button edges. A later gesture requires a fresh cursor observation.
- Feed the existing `ViewportNavigationInput::record_primary_edge`; its 64-edge cap, ordered transition queue, `PointClick::StampedPress` camera/view protection, async GPU stale-readback checks and canonical `SceneAnchor` identity remain unchanged.
- Maintain the usual `ViewportNavigationInput` frame-end pointer, modifiers and buttons for camera navigation. The event-local override is only for recording each primary edge.

## Tests and adoption

The existing `new_stream_generation_rejects_old_pointer_mapping` test drives the Bevy update system with unified `WindowEvent` messages. It checks two distinct click positions and changing Ctrl state within one update, cursor exit, focus loss and regain, and replacement of the primary window. The test uses the existing registered identity.

The corrected adapter consumes the order published by Bevy Winit. The update test covers that message-to-intent boundary. Actual OS ingress into a standalone Bevy window requires a host that enables this adapter.
