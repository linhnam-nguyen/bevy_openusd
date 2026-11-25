use super::*;
use bevy::asset::{Assets, RenderAssetUsages};
use bevy::ecs::world::CommandQueue;
use bevy::mesh::{Mesh, PrimitiveTopology};
use usd_model::{Bounds3, HashDigest};
use usd_project::{SceneId, SceneMemberId};

fn payload_with_hash(
    scene_id: SceneId,
    occurrence: SceneMemberId,
    hash: u8,
) -> SceneSpatialPayload {
    SceneSpatialPayload {
        address: SceneCacheAddress {
            scene_id,
            occurrence: crate::cache::contract::SceneCacheOccurrence::Member(occurrence),
        },
        payload_key: ScenePayloadKey {
            scene_id,
            blob_hash: HashDigest::new([hash; HashDigest::BYTE_LEN]),
        },
        transform: ScenePlacementTransform::IDENTITY,
        bounds: Bounds3 {
            min: [-1.0; 3],
            max: [1.0; 3],
        },
        cpu_bytes: 8,
        gpu_bytes: 8,
    }
}

fn payload(scene_id: SceneId, occurrence: SceneMemberId) -> SceneSpatialPayload {
    payload_with_hash(scene_id, occurrence, 7)
}

#[test]
fn shared_mesh_handle_projects_each_occurrence_and_retirement_removes_all() {
    let scene = SceneId::new_v4();
    let first = payload(scene, SceneMemberId::new_v4());
    let second = payload(scene, SceneMemberId::new_v4());
    let key = first.payload_key;
    let hidden = first.address.clone();
    let unrelated = (8u8..72u8)
        .map(|hash| payload_with_hash(scene, SceneMemberId::new_v4(), hash))
        .collect::<Vec<_>>();
    let mut payloads = vec![first, second];
    payloads.extend(unrelated);
    let mut projection = SceneResidencyProjection::default();
    projection.install_scene(&payloads);

    let mut world = World::new();
    world.insert_resource(Assets::<Mesh>::default());
    let handle = world.resource_mut::<Assets<Mesh>>().add(Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    ));
    let mut queue = CommandQueue::default();
    {
        let mut commands = Commands::new(&mut queue, &world);
        projection.attach_payload(key, handle.clone(), &mut commands, |address| {
            *address != hidden
        });
    }
    queue.apply(&mut world);
    assert_eq!(projection.entry_count(), 66);
    assert_eq!(projection.active_entity_count(), 2);
    assert_eq!(
        projection.payload_occurrences.get(&key).map(Vec::len),
        Some(2)
    );
    assert_eq!(
        projection.asset_occurrences.get(&handle.id()).map(Vec::len),
        Some(2)
    );
    let mut query = world.query::<(&Mesh3d, &SceneResidencyOccurrence)>();
    assert_eq!(query.iter(&world).count(), 2);
    assert!(query.iter(&world).all(
        |(mesh, occurrence)| mesh.0.id() == handle.id() && occurrence.address.scene_id == scene
    ));
    let mut visibility = world.query::<(&Visibility, &SceneResidencyOccurrence)>();
    assert!(visibility.iter(&world).all(|(value, occurrence)| {
        (*value == Visibility::Hidden) == (occurrence.address == hidden)
    }));

    let mut queue = CommandQueue::default();
    projection.sync_visibility(&mut Commands::new(&mut queue, &world), |_| true);
    queue.apply(&mut world);
    assert!(
        visibility
            .iter(&world)
            .all(|(value, _)| *value == Visibility::Visible)
    );

    let mut queue = CommandQueue::default();
    {
        let mut commands = Commands::new(&mut queue, &world);
        projection.release_asset(handle.id(), &mut commands);
    }
    queue.apply(&mut world);
    assert_eq!(projection.active_entity_count(), 0);
    assert_eq!(projection.entry_count(), 66);
    assert_eq!(
        projection.payload_occurrences.get(&key).map(Vec::len),
        Some(2)
    );
    assert!(!projection.asset_occurrences.contains_key(&handle.id()));
    assert_eq!(
        world
            .query::<&SceneResidencyOccurrence>()
            .iter(&world)
            .count(),
        0
    );
    cached_occurrences_follow_live_visibility();
}

fn cached_occurrences_follow_live_visibility() {
    use super::super::{
        ResidencyAuthority, ResidencyReason, VisibilityChangeEpoch, advance_visibility_epoch,
        pump_residency_uploads, update_camera_residency,
    };
    use crate::cache::contract::SceneCacheOccurrence;
    use crate::viewport::rendering::scene::SceneAnchorIndex;
    use bevy::prelude::*;
    use bevy::render::render_asset::RenderAssetBytesPerFrame;
    use viewport_protocol::SceneAnchor;

    let scene = SceneId::new_v4();
    let mut first = payload(scene, SceneMemberId::new_v4());
    first.address.occurrence = SceneCacheOccurrence::PrimPath("/First".into());
    let mut second = payload(scene, SceneMemberId::new_v4());
    second.address.occurrence = SceneCacheOccurrence::PrimPath("/Second".into());
    let key = first.payload_key;
    let hidden_address = first.address.clone();
    let payloads = vec![first, second];
    let mut world = World::new();
    let parent = world.spawn(Visibility::Hidden).id();
    let first = world.spawn((Visibility::Inherited, ChildOf(parent))).id();
    let second = world.spawn((Visibility::Visible, ChildOf(parent))).id();
    world.spawn((Camera3d::default(), GlobalTransform::from_xyz(0., 0., 10.)));
    world.insert_resource(SceneAnchorIndex::from_test_entities(vec![
        (SceneAnchor::active_session("/First"), first),
        (SceneAnchor::active_session("/Second"), second),
    ]));
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, 1, payloads.clone());
    world.insert_resource(authority);
    world.init_resource::<VisibilityChangeEpoch>();
    world.insert_resource(RenderAssetBytesPerFrame::new(0));
    let mut assets = Assets::<Mesh>::default();
    let handle = assets.add(Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    ));
    world.insert_resource(assets);
    let mut projection = SceneResidencyProjection::default();
    projection.install_scene(&payloads);
    let mut queue = CommandQueue::default();
    projection.attach_payload(
        key,
        handle.clone(),
        &mut Commands::new(&mut queue, &world),
        |address| *address != hidden_address,
    );
    queue.apply(&mut world);
    world.insert_resource(projection);
    let mut residency = Schedule::default();
    residency.add_systems(
        (
            advance_visibility_epoch,
            update_camera_residency,
            pump_residency_uploads,
        )
            .chain(),
    );
    residency.run(&mut world);
    let mut surfaces = world.query::<(&Mesh3d, &SceneResidencyOccurrence, &Visibility)>();
    assert!(
        world
            .resource::<ResidencyAuthority>()
            .reasons(&key)
            .is_some_and(|reasons| reasons.contains(&ResidencyReason::CameraNear))
    );
    *world.get_mut::<Visibility>(second).unwrap() = Visibility::Inherited;
    residency.run(&mut world);
    assert!(surfaces.iter(&world).all(
        |(mesh, _, visibility)| mesh.0.id() == handle.id() && *visibility == Visibility::Hidden
    ));
    assert!(
        world
            .resource::<ResidencyAuthority>()
            .reasons(&key)
            .is_none_or(|reasons| !reasons.contains(&ResidencyReason::CameraNear))
    );
    world.entity_mut(parent).remove::<Visibility>();
    residency.run(&mut world);
    assert!(
        surfaces
            .iter(&world)
            .all(|(_, _, visibility)| *visibility == Visibility::Visible)
    );
    let hidden_parent = world.spawn(Visibility::Hidden).id();
    world.entity_mut(first).insert(ChildOf(hidden_parent));
    // A missing local Visibility inherits from the new parent, as in the SDK route.
    world.entity_mut(first).remove::<Visibility>();
    residency.run(&mut world);
    assert!(surfaces.iter(&world).all(|(mesh, occurrence, visibility)| {
        mesh.0.id() == handle.id()
            && (*visibility == Visibility::Hidden) == (occurrence.address == hidden_address)
    }));
    world.entity_mut(first).remove::<ChildOf>();
    residency.run(&mut world);
    assert_eq!(surfaces.iter(&world).count(), 2);
    assert!(
        surfaces
            .iter(&world)
            .all(|(_, _, visibility)| *visibility == Visibility::Visible)
    );
    assert_eq!(world.resource::<SceneAnchorIndex>().revision(), 1);
    let epoch = world.resource::<VisibilityChangeEpoch>().0;
    for _ in 0..100 {
        residency.run(&mut world);
    }
    assert_eq!(world.resource::<VisibilityChangeEpoch>().0, epoch);
}

#[test]
fn projection_uses_resolved_scene_space_transform() {
    let scene = SceneId::new_v4();
    let mut occurrence = payload(scene, SceneMemberId::new_v4());
    occurrence.transform = ScenePlacementTransform::from_trs(
        [10.0, 2.0, 0.0],
        [
            std::f64::consts::FRAC_1_SQRT_2,
            0.0,
            0.0,
            std::f64::consts::FRAC_1_SQRT_2,
        ],
        [2.0, 1.0, 1.0],
    );
    let mut projection = SceneResidencyProjection::default();
    projection.install_scene(&[occurrence]);

    let mut world = World::new();
    world.insert_resource(Assets::<Mesh>::default());
    let handle = world.resource_mut::<Assets<Mesh>>().add(Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    ));
    let mut queue = CommandQueue::default();
    {
        let mut commands = Commands::new(&mut queue, &world);
        projection.attach_payload(
            ScenePayloadKey {
                scene_id: scene,
                blob_hash: HashDigest::new([7; HashDigest::BYTE_LEN]),
            },
            handle,
            &mut commands,
            |_| true,
        );
    }
    queue.apply(&mut world);
    let mut query = world.query::<&Transform>();
    let transform = query.single(&world).expect("one projected transform");
    assert_eq!(transform.translation, bevy::math::Vec3::new(10.0, 2.0, 0.0));
    assert_eq!(transform.scale, bevy::math::Vec3::new(2.0, 1.0, 1.0));
}
