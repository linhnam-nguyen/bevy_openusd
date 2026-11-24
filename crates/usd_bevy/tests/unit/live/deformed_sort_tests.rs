use super::*;
use bevy::ecs::system::RunSystemOnce;
use bevy::render::{
    render_phase::{DrawFunctionId, PhaseItemExtraIndex, SortedRenderPhase},
    render_resource::CachedRenderPipelineId,
};

#[test]
fn first_queued_skinned_draw_uses_current_center_and_restores_after_removal() {
    let mut world = World::new();
    let skinned = world.spawn_empty().id();
    let other = world.spawn_empty().id();
    let view = RetainedViewEntity {
        main_entity: world.spawn_empty().id().into(),
        auxiliary_entity: None,
        subview_index: 0,
    };
    let mut phase = SortedRenderPhase::default();
    for id in [skinned, other] {
        phase.add_retained(Transparent3d {
            sorting_info: TransparentSortingInfo3d::Sorted {
                mesh_center: Vec3::ZERO,
                depth_bias: 0.5,
            },
            distance: 0.0,
            pipeline: CachedRenderPipelineId::INVALID,
            entity: (Entity::PLACEHOLDER, id.into()),
            draw_function: DrawFunctionId(0),
            batch_range: 0..1,
            extra_index: PhaseItemExtraIndex::None,
            indexed: false,
        });
    }
    let mut phases = ViewSortedRenderPhases::default();
    phases.0.insert(view, phase);
    world.insert_resource(phases);
    world.init_resource::<Overrides>();
    world.insert_resource(Centers([(skinned.into(), Vec3::Z)].into_iter().collect()));

    let assert_center = |world: &World, id: Entity, expected: Vec3| {
        let phases = world.resource::<ViewSortedRenderPhases<Transparent3d>>();
        let item = &phases.0[&view].items[&(Entity::PLACEHOLDER, id.into())];
        let TransparentSortingInfo3d::Sorted {
            mesh_center,
            depth_bias,
        } = item.sorting_info
        else {
            panic!("sorting mode changed")
        };
        assert_eq!(mesh_center, expected);
        assert_eq!(depth_bias, 0.5);
    };

    world.run_system_once(apply_centers).unwrap();
    assert_center(&world, skinned, Vec3::Z);
    assert_center(&world, other, Vec3::ZERO);
    world.run_system_once(restore_centers).unwrap();
    world.resource_mut::<Centers>().0.clear();
    world.run_system_once(apply_centers).unwrap();
    assert_center(&world, skinned, Vec3::ZERO);
    assert_center(&world, other, Vec3::ZERO);
}
