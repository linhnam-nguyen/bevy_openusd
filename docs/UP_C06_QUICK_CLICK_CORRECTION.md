# UP-C06 quick-click delivery: source-level contract

## Failure repaired (F-C06-I2-01)

A press followed by a release in one renderer update interval is a valid click.
`ViewportNavigationInput.buttons.primary` is *not* an event stream and cannot
be used by itself to infer all button transitions. This correction retains
ordered `PrimaryButtonEdge` records until `select_point_click` drains them.

The browser sends a zero-delta `PointerMotion` carrying the event's actual CSS
coordinates and viewport size on the **reliable** input path immediately before
its `ButtonState` transition. The host queues reliable pointer snapshots and
button transitions together. A newer unordered motion packet can update hover
without overriding a button edge's reliable snapshot. No wire schema change
is introduced.

Mara forwards ordered `egui::Event::PointerButton` records only for its
viewport interaction layer. The Bevy window adapter reads ordered native mouse
button messages. The per-frame motion reset preserves the edge queue. Focus
loss, UI capture, generation changes, and `ReleaseAll` invalidate queued
gestures. Queue overflow cancels input. Idle timeout releases held buttons;
an already released tap can finish a multi-batch GPU traversal after traffic
goes idle.

## Selection and GPU correlation

`PointClick` owns a bounded pending-release queue. Each release carries its
actual pointer/viewport position and the stream/scene generation, and gets
its own sequence. `update_hover_target` resolves the *front* queued release
before normal hover, including one full GPU pick traversal when applicable.
Only the matching click sequence, position, viewport size and current scene
may yield a canonical `ViewportCommand`. Stale GPU callbacks cannot select
some other cursor's target. Success and explicit GPU readback failures retain
the existing distinction. Logical IDs, selection caps, Section Box geometry,
presentation authority and document state are unchanged.

## Regression cases

Exercise press and release in one update, rapid taps at different coordinates,
and a queued tap that spans multiple GPU batches. The input queue also needs
coverage for focus loss, stream generation changes, replacement by a newer
unordered cursor position, box selection, and clipped geometry.
