use bevy::prelude::App;

pub(super) fn wait_for_manifest(
    app: &mut App,
    interface: &viewport_streaming::RenderServerInterface,
    previous_revision: Option<&str>,
) -> viewport_protocol::RuntimeManifest {
    for _ in 0..100 {
        if let Some(manifest) = interface.runtime_manifest()
            && previous_revision.is_none_or(|previous| manifest.revision != previous)
        {
            return manifest;
        }
        app.update();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("runtime delivery worker did not publish a manifest");
}
