use anyhow::Result;
use bevy::prelude::{GlobalTransform, Transform, Vec3};

use super::native_instance_review::{nested_consumers_stage, projected_app, projected_entity};
use crate::author_transform;
use crate::live::{LiveStage, PendingStageChanges};

#[test]
fn native_instance_prototype_ancestor_change_patches_only_descendant_consumers() -> Result<()> {
    let mut app = projected_app(nested_consumers_stage());
    let leaf_a = projected_entity(&app, "/Outer_A/Nested/Leaf");
    let leaf_b = projected_entity(&app, "/Outer_B/Nested/Leaf");
    let other_leaf = projected_entity(&app, "/Other_C/OtherLeaf");
    let before_a = app
        .world()
        .get::<GlobalTransform>(leaf_a)
        .expect("Outer_A leaf global transform")
        .compute_transform()
        .translation;
    let before_b = app
        .world()
        .get::<GlobalTransform>(leaf_b)
        .expect("Outer_B leaf global transform")
        .compute_transform()
        .translation;
    let before_other = app
        .world()
        .get::<GlobalTransform>(other_leaf)
        .expect("Other_C leaf global transform")
        .compute_transform()
        .translation;

    {
        let live = app.world().get_non_send::<LiveStage>().expect("live stage");
        author_transform(
            &live.stage,
            "/OuterPrototype/Nested",
            &Transform::from_translation(Vec3::new(0.0, 5.0, 0.0)),
        )?;
    }

    app.update();

    let pending = app
        .world()
        .resource::<PendingStageChanges>()
        .batch()
        .expect("prototype property change is queued");
    assert!(
        !pending.has_resync(),
        "ancestor transform is an ordinary change"
    );
    assert!(
        pending
            .changes
            .iter()
            .flat_map(|change| change.changed_info.iter())
            .any(|path| path.starts_with("/OuterPrototype/Nested")),
        "ancestor transform change reaches the live stage-change sink"
    );
    let after_a = app
        .world()
        .get::<GlobalTransform>(leaf_a)
        .expect("Outer_A leaf global transform after patch")
        .compute_transform()
        .translation;
    let after_b = app
        .world()
        .get::<GlobalTransform>(leaf_b)
        .expect("Outer_B leaf global transform after patch")
        .compute_transform()
        .translation;
    let after_other = app
        .world()
        .get::<GlobalTransform>(other_leaf)
        .expect("Other_C leaf global transform after patch")
        .compute_transform()
        .translation;
    assert_ne!(after_a, before_a, "Outer_A descendant consumer was patched");
    assert_ne!(after_b, before_b, "Outer_B descendant consumer was patched");
    assert_eq!(
        after_other, before_other,
        "unrelated instance branch is untouched"
    );
    Ok(())
}
