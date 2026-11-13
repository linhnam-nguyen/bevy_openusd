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
            occurrence: crate::project::cache_contract::SceneCacheOccurrence::Member(occurrence),
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
        projection.attach_payload(key, handle.clone(), &mut commands);
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
        );
    }
    queue.apply(&mut world);
    let mut query = world.query::<&Transform>();
    let transform = query.single(&world).expect("one projected transform");
    assert_eq!(transform.translation, bevy::math::Vec3::new(10.0, 2.0, 0.0));
    assert_eq!(transform.scale, bevy::math::Vec3::new(2.0, 1.0, 1.0));
}
