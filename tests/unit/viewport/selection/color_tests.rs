use super::*;
use viewport_protocol::SelectionPresentationSettings;

#[test]
fn selection_color_preserves_rgb_channels() {
    let color = color_from_rgb8(ColorRgb8::new(0x34, 0xA0, 0xF2));
    assert_eq!(color.to_srgba().to_u8_array(), [0x34, 0xA0, 0xF2, 0xFF]);
}

#[test]
fn default_selection_color_override_is_disabled_but_has_documented_color() {
    let settings = SelectionPresentationSettings::default();
    assert!(!settings.color_change_enabled);
    assert_eq!(
        color_from_rgb8(settings.selection_color)
            .to_srgba()
            .to_u8_array(),
        [0x38, 0xBD, 0xF8, 0xFF]
    );
}

#[test]
fn selection_color_has_priority_over_hover_color() {
    assert_eq!(
        presentation_owner(true, true),
        Some(PresentationOwner::Selection)
    );
    assert_eq!(
        presentation_owner(false, true),
        Some(PresentationOwner::Hover)
    );
}
