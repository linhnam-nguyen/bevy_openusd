use super::collect_skeleton_runtime;
use crate::{UsdPrimRef, UsdSkelAnimDriver, snippet::UsdSnippet};
use bevy::prelude::*;
use openusd::usd::TimeCode;

#[test]
fn references_share_pose_only_when_composed_specs_and_offsets_match() {
    let stage = UsdSnippet::new(
        r#"#usda 1.0
def Xform "Source" {
    def Skeleton "Skel" {}
    def SkelAnimation "Anim" {
        uniform token[] joints = ["Root"]
        float3[] translations.timeSamples = { 0: [(1.0, 0.0, 0.0)], 20: [(21.0, 0.0, 0.0)] }
        quatf[] rotations = [(1.0, 0.0, 0.0, 0.0)]
        half3[] scales = [(1.0, 1.0, 1.0)]
    }
}
def Xform "A" (prepend references = </Source>) {}
def Xform "B" (prepend references = </Source>) {}
def Xform "Offset" (prepend references = </Source> (offset = 10)) {}
def Xform "Override" (prepend references = </Source>) {
    over "Anim" { float3[] translations = [(7.0, 0.0, 0.0)] }
}
"#,
    )
    .open_stage()
    .unwrap();
    let mut world = World::new();
    for root in ["/A", "/B", "/Offset", "/Override"] {
        world.spawn((
            UsdPrimRef::new(format!("{root}/Skel")),
            UsdSkelAnimDriver {
                animation_source_path: format!("{root}/Anim"),
                ..default()
            },
        ));
    }

    let poses = collect_skeleton_runtime(&mut world, &stage);
    assert_eq!(poses.len(), 3);
    assert_eq!(
        poses
            .iter()
            .filter(|pose| pose.skeletons.len() == 2)
            .count(),
        1
    );
    let mut translations = poses
        .iter()
        .map(|pose| {
            let (trans, _, _) = pose
                .animation
                .joint_transform_components(TimeCode::new(10.0))
                .unwrap()
                .unwrap();
            trans[0].x
        })
        .collect::<Vec<_>>();
    translations.sort_by(f32::total_cmp);
    assert_eq!(translations, [1.0, 7.0, 11.0]);
}
