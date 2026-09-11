use bevy::prelude::World;

use crate::live::{
    LiveStage, PathStore, PerformanceCounters, PrimEntities, extract_render_payloads_for_paths,
    project_paths,
};
use crate::snippet::UsdSnippet;

#[test]
fn exact_path_projection_materializes_only_requested_namespace() {
    let stage = UsdSnippet::new(
        r#"#usda 1.0

def Xform "A"
{
    def Xform "Leaf"
    {
    }
}
def Xform "B"
{
}
"#,
    )
    .open_stage()
    .expect("targeted stage opens");
    let live = LiveStage::new(stage);
    let mut world = World::new();
    world.insert_resource(PerformanceCounters {
        enabled: true,
        ..Default::default()
    });
    let mut map = PrimEntities::default();

    let materialized = project_paths(&mut world, &live, &mut map, &["/A/Leaf"])
        .expect("targeted projection succeeds");
    let paths = world.resource::<PathStore>();
    assert_eq!(materialized, 3);
    assert!(map.entity(paths, "/").is_some());
    assert!(map.entity(paths, "/A").is_some());
    assert!(map.entity(paths, "/A/Leaf").is_some());
    assert!(map.entity(paths, "/B").is_none());
    assert_eq!(
        world
            .resource::<PerformanceCounters>()
            .projection_full_stage_walks,
        0
    );
}

#[test]
fn targeted_extraction_reads_only_requested_meshes() {
    let stage = UsdSnippet::new(
        r#"#usda 1.0

def Mesh "A"
{
    point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0, 1, 2]
}
def Mesh "B"
{
    point3f[] points = [(0, 0, 0), (2, 0, 0), (0, 2, 0)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0, 1, 2]
}
"#,
    )
    .open_stage()
    .expect("targeted mesh stage opens");

    let payloads =
        extract_render_payloads_for_paths(&stage, &["/A"]).expect("targeted extraction succeeds");
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0].path, "/A");
    assert!(
        payloads[0]
            .mesh
            .attribute(bevy::mesh::Mesh::ATTRIBUTE_POSITION)
            .is_some()
    );
}
