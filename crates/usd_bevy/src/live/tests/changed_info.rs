use bevy::prelude::*;

use crate::live::{
    LiveRevision, LiveStage, LiveStagePlugin, PathStore, PrimEntities, StageChange,
    StageChangeBatch, apply_change_batch, author_transform,
};
use crate::snippet::UsdSnippet;

#[test]
fn test_bare_prim_changed_info_repatches_transform() {
    let usda = r#"#usda 1.0
def Xform "World"
{
    def Xform "A"
    {
        double3 xformOp:translate = (0, 0, 0)
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }
}
"#;
    let stage = UsdSnippet::new(usda).open_stage().expect("stage opens");
    let live = LiveStage::new(stage);

    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(live);
    app.update();

    let entity = app
        .world()
        .resource::<PrimEntities>()
        .entity(app.world().resource::<PathStore>(), "/World/A")
        .unwrap();
    let initial_transform = *app.world().get::<Transform>(entity).unwrap();
    assert_eq!(initial_transform.translation, Vec3::ZERO);

    // Author a new translation on /World/A
    {
        let live = app.world().get_non_send::<LiveStage>().unwrap();
        author_transform(
            &live.stage,
            "/World/A",
            &Transform::from_translation(Vec3::new(10.0, 20.0, 30.0)),
        )
        .unwrap();
        let _ = live.drain_change_batch();
    }

    // Manually submit a bare prim changed_info (no .property suffix)
    let live = app.world_mut().remove_non_send::<LiveStage>().unwrap();
    let mut map = app.world_mut().remove_resource::<PrimEntities>().unwrap();
    let batch = StageChangeBatch {
        revision: LiveRevision(2),
        changes: vec![StageChange {
            resynced: Vec::new(),
            changed_info: vec!["/World/A".to_string()],
        }],
    };
    apply_change_batch(app.world_mut(), &live, &mut map, &batch);
    app.world_mut().insert_non_send(live);
    app.world_mut().insert_resource(map);

    // Verify Transform component was repatched to the new translation
    let updated_transform = *app.world().get::<Transform>(entity).unwrap();
    assert_eq!(
        updated_transform.translation,
        Vec3::new(10.0, 20.0, 30.0),
        "bare prim changed_info must repatch transform"
    );
}
