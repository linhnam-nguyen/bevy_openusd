//! Current pure `usd_rapier` builder smoke test.

use rapier3d_f64::glamx::{DQuat, DVec3};
use rapier3d_f64::prelude::{ColliderSet, RigidBodySet};
use usd_rapier::bodies::{RigidBodyOpinion, build_rigid_body};
use usd_rapier::colliders::{
    ColliderOpinion, ShapeInput, build_collider, stage_collider_interaction_groups,
};

fn open_usda_stage(tag: &str, usda: &str) -> openusd::usd::Stage {
    let path = std::env::temp_dir().join(format!("{}_{}", std::process::id(), tag));
    std::fs::write(&path, usda).expect("write temp usda");
    let stage = openusd::usd::Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .open(path.to_str().expect("utf8 path"))
        .expect("open stage from temp file");
    let _ = std::fs::remove_file(&path);
    stage
}

#[test]
fn current_builders_insert_a_body_and_attached_collider() {
    let mut bodies = RigidBodySet::new();
    let mut colliders = ColliderSet::new();
    let body = build_rigid_body(
        &mut bodies,
        &RigidBodyOpinion {
            kinematic: false,
            enabled: true,
            starts_asleep: false,
            world_translation: DVec3::ZERO,
            world_rotation: DQuat::IDENTITY,
            linvel: DVec3::ZERO,
            angvel: DVec3::ZERO,
            mass: Some(1.0),
            center_of_mass: None,
            diagonal_inertia: None,
            principal_axes: None,
        },
        7,
    )
    .expect("body builds");
    let collider = build_collider(
        &mut colliders,
        &mut bodies,
        Some(body),
        ColliderOpinion {
            shape: ShapeInput::Cube { size: 1.0 },
            local_pose: rapier3d_f64::math::Pose {
                translation: DVec3::ZERO,
                rotation: DQuat::IDENTITY,
            },
            friction: None,
            restitution: None,
            collision_groups: None,
            user_data: 8,
        },
    )
    .expect("collider builds")
    .expect("cube produces a collider");

    assert!(bodies.get(body).is_some());
    assert!(colliders.get(collider).is_some());

    // F-C03-I1-03 (1): Positive 2-group symmetric collision filtering.
    let two_group_stage = open_usda_stage(
        "f03_two_groups.usda",
        r#"#usda 1.0
def Xform "World" {
    def Cube "BoxA" {}
    def Cube "BoxB" {}
    def PhysicsCollisionGroup "GroupA" (
        prepend apiSchemas = ["CollectionAPI:colliders"]
    ) {
        rel collection:colliders:includes = [</World/BoxA>]
        rel physics:filteredGroups = [</World/GroupB>]
    }
    def PhysicsCollisionGroup "GroupB" (
        prepend apiSchemas = ["CollectionAPI:colliders"]
    ) {
        rel collection:colliders:includes = [</World/BoxB>]
    }
}
"#,
    );
    let two_map =
        stage_collider_interaction_groups(&two_group_stage).expect("2 symmetric groups succeed");
    let box_a = openusd::sdf::path("/World/BoxA").unwrap();
    let box_b = openusd::sdf::path("/World/BoxB").unwrap();
    assert!(
        two_map[&box_a].test(two_map[&box_a]),
        "BoxA collides with its own group"
    );
    assert!(
        !two_map[&box_a].test(two_map[&box_b]),
        "filtered GroupA and GroupB do not collide"
    );

    // F-C03-I1-03 (2): Boundary 32 groups, verifying the 32nd group's collider bit and filter.
    let mut usda_32 = String::from("#usda 1.0\ndef Xform \"World\" {\n");
    for i in 0..32 {
        usda_32.push_str(&format!("    def Cube \"Box{i}\" {{}}\n"));
        usda_32.push_str(&format!(
            "    def PhysicsCollisionGroup \"Group{i}\" (prepend apiSchemas = [\"CollectionAPI:colliders\"]) {{\n        rel collection:colliders:includes = [</World/Box{i}>]\n    }}\n"
        ));
    }
    usda_32.push_str("}\n");
    let stage_32 = open_usda_stage("f03_32_groups.usda", &usda_32);
    let map_32 = stage_collider_interaction_groups(&stage_32).expect("32 groups succeed");
    let box_31 = openusd::sdf::path("/World/Box31").unwrap();
    assert_eq!(
        map_32[&box_31].memberships.bits(),
        1u32 << 31,
        "32nd group uses bit 31"
    );
    assert!(
        map_32[&box_31].test(map_32[&openusd::sdf::path("/World/Box0").unwrap()]),
        "32nd group collider interacts with 1st group collider"
    );

    // F-C03-I1-03 (3): Negative 33 groups must return Err, never silently truncate to 32.
    let mut usda_33 = String::from("#usda 1.0\ndef Xform \"World\" {\n");
    for i in 0..33 {
        usda_33.push_str(&format!("    def Cube \"Box{i}\" {{}}\n"));
        usda_33.push_str(&format!(
            "    def PhysicsCollisionGroup \"Group{i}\" (prepend apiSchemas = [\"CollectionAPI:colliders\"]) {{\n        rel collection:colliders:includes = [</World/Box{i}>]\n    }}\n"
        ));
    }
    usda_33.push_str("}\n");
    let stage_33 = open_usda_stage("f03_33_groups.usda", &usda_33);
    assert!(
        stage_collider_interaction_groups(&stage_33).is_err(),
        "33 collision groups must return Err instead of truncating to 32"
    );

    // F-C03-I1-03 (4): Overlapping multi-group membership verified against the
    // native USD CollisionGroupTable pairwise oracle.
    let overlap_stage = open_usda_stage(
        "f03_overlap_groups.usda",
        r#"#usda 1.0
def Xform "World" {
    def Cube "BoxOverlap" {}
    def Cube "BoxB" {}
    def Cube "BoxC" {}
    def PhysicsCollisionGroup "GroupA" (
        prepend apiSchemas = ["CollectionAPI:colliders"]
    ) {
        rel collection:colliders:includes = [</World/BoxOverlap>]
        rel physics:filteredGroups = [</World/GroupB>]
    }
    def PhysicsCollisionGroup "GroupB" (
        prepend apiSchemas = ["CollectionAPI:colliders"]
    ) {
        rel collection:colliders:includes = [</World/BoxB>]
        rel physics:filteredGroups = [</World/GroupC>]
    }
    def PhysicsCollisionGroup "GroupC" (
        prepend apiSchemas = ["CollectionAPI:colliders"]
    ) {
        rel collection:colliders:includes = [</World/BoxOverlap>, </World/BoxC>]
    }
}
"#,
    );
    let overlap_map = stage_collider_interaction_groups(&overlap_stage)
        .expect("overlapping multi-group membership verified against USD pairwise oracle");
    let box_overlap = openusd::sdf::path("/World/BoxOverlap").unwrap();
    let box_b = openusd::sdf::path("/World/BoxB").unwrap();
    let box_c = openusd::sdf::path("/World/BoxC").unwrap();
    assert!(
        !overlap_map[&box_overlap].test(overlap_map[&box_b]),
        "BoxOverlap (in GroupA and GroupC, both filtered against GroupB) does not collide with BoxB"
    );
    assert!(
        overlap_map[&box_overlap].test(overlap_map[&box_c]),
        "BoxOverlap collides with BoxC via shared/unfiltered GroupA/GroupC"
    );
}
