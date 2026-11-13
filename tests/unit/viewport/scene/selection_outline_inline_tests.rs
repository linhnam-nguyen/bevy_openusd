use super::*;

#[test]
fn selection_outline_color_preserves_rgb_channels() {
    let color = color_from_rgb8(ColorRgb8::new(0x12, 0x80, 0xF0));
    assert_eq!(color.to_srgba().to_u8_array(), [0x12, 0x80, 0xF0, 0xFF]);
}

#[test]
fn default_selection_boundary_uses_the_documented_width() {
    let settings = SelectionPresentationSettings::default();
    let outline = OutlineVolume {
        visible: settings.boundary_enabled,
        width: SELECTION_OUTLINE_WIDTH,
        colour: color_from_rgb8(settings.boundary_color),
    };
    assert!(outline.visible);
    assert_eq!(outline.width, 3.0);
}
