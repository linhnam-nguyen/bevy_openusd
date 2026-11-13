use super::*;

#[test]
fn headless_egui_is_passive_without_removing_shared_input() {
    let mut app = App::new();
    app.init_resource::<Assets<bevy::shader::Shader>>();
    app.add_plugins(bevy_egui::EguiPlugin::default());

    configure_headless_egui(&mut app);

    let settings = app.world().resource::<bevy_egui::EguiGlobalSettings>();
    assert!(!settings.auto_create_primary_context);
    assert!(!settings.enable_ime);
    assert!(!settings.enable_cursor_icon_updates);
    assert!(
        app.world()
            .contains_resource::<bevy_egui::input::EguiWantsInput>()
    );
}
