//! Current live-stage projection smoke test.

use bevy::prelude::World;
use usd_bevy::{LiveStage, PrimEntities, UsdPrimRef, UsdSnippet, project_stage};

#[test]
fn current_live_projection_builds_a_prim_entity_map() {
    let stage = UsdSnippet::new(
        r#"#usda 1.0

def Xform "Root"
{
    def Xform "Child"
    {
    }
}
"#,
    )
    .open_stage()
    .expect("live fixture opens");
    let live = LiveStage::new(stage);
    let mut world = World::new();
    let mut map = PrimEntities::default();

    project_stage(&mut world, &live, &mut map);

    let root = map.entity("/Root").expect("Root is projected");
    let child = map.entity("/Root/Child").expect("Child is projected");
    assert_ne!(root, child);
    assert_eq!(world.get::<UsdPrimRef>(root).unwrap().path, "/Root");
    assert_eq!(world.get::<UsdPrimRef>(child).unwrap().path, "/Root/Child");
}
