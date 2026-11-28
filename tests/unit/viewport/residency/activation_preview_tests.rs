use super::{CachePinGuard, Camera, LiveStage, SceneCacheStore, WarmScenePreview, World};
use crate::cache::blobs::BlobStore;

#[test]
fn only_the_current_preview_can_restore_the_retained_stage_and_camera() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let store = SceneCacheStore::new(directory.path());
    let blob = store
        .shared_object_store()?
        .put(b"pinned preview fixture")?;
    let pin = CachePinGuard::acquire(directory.path(), Vec::new(), vec![blob])?;
    let mut world = World::new();
    let stage = LiveStage::new(openusd::usd::Stage::builder().in_memory("preview.usda")?);
    let retained = stage.session_id();
    assert!(stage.try_freeze_authoring());
    world.insert_non_send(stage);
    let previous_camera = world
        .spawn(Camera {
            is_active: false,
            ..Camera::default()
        })
        .id();
    let preview_camera = world.spawn(Camera::default()).id();
    let preview_geometry = world.spawn_empty().id();
    world.insert_resource(WarmScenePreview {
        activation: 7,
        camera: preview_camera,
        previous_session: retained,
        expires: std::time::Instant::now(),
        cameras: vec![previous_camera],
        entities: vec![preview_camera, preview_geometry],
        meshes: Vec::new(),
        _pin: pin,
    });

    assert!(WarmScenePreview::take(&mut world, 6).is_none());
    assert!(world.non_send::<LiveStage>().is_authoring_frozen());
    assert!(!world.get::<Camera>(previous_camera).unwrap().is_active);
    assert!(world.get_entity(preview_geometry).is_ok());

    assert!(WarmScenePreview::take(&mut world, 7).is_some());
    assert_eq!(world.non_send::<LiveStage>().session_id(), retained);
    assert!(!world.non_send::<LiveStage>().is_authoring_frozen());
    assert!(world.get::<Camera>(previous_camera).unwrap().is_active);
    assert!(world.get_entity(preview_camera).is_err());
    assert!(world.get_entity(preview_geometry).is_err());
    Ok(())
}
