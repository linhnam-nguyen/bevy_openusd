use bevy::prelude::*;

use crate::live::{LiveStage, LiveStagePlugin, PathStore, PrimEntities};
use crate::snippet::UsdSnippet;

#[test]
fn test_live_stage_load_and_unload_payload_api_preserves_projection() {
    let usda = r#"#usda 1.0
def Xform "World"
{
    def "PayloadPrim" (
        payload = @./sub.usda@</Sub>
    )
    {
    }
}
"#;
    let stage = UsdSnippet::new(usda)
        .open_stage()
        .expect("payload stage opens");
    let live = LiveStage::new(stage);
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(live);
    app.update();

    let prim_entities = app.world().resource::<PrimEntities>();
    assert!(
        prim_entities
            .entity(app.world().resource::<PathStore>(), "/World/PayloadPrim")
            .is_some()
    );

    let live = app.world().get_non_send::<LiveStage>().unwrap();
    live.unload_payload("/World/PayloadPrim");
    live.load_payload("/World/PayloadPrim");
}
