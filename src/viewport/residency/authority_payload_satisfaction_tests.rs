use super::*;

use bevy::asset::Assets;
use bevy::mesh::Mesh;
use usd_model::HashDigest;

fn key(scene_id: SceneId, value: u8) -> ScenePayloadKey {
    ScenePayloadKey {
        scene_id,
        blob_hash: HashDigest::new([value; HashDigest::BYTE_LEN]),
    }
}

fn resident_non_animation_payload(
    authority: &mut ResidencyAuthority,
    assets: &mut Assets<Mesh>,
    key: ScenePayloadKey,
    generation: u64,
) -> bevy::asset::AssetId<Mesh> {
    assert!(authority.request_reason(
        key,
        ResidencyReason::Selected,
        generation,
        8,
        8,
    ));
    let _job = authority.begin_next_load().expect("geometry load");
    assert!(authority.complete_cpu(key, generation, 8, 8));
    assert_eq!(authority.pump_uploads(assets, None), vec![key]);
    authority.render_asset_id(&key).expect("resident mesh")
}

#[test]
fn animation_demand_queues_supplement_without_replacing_resident_mesh() {
    let scene = SceneId::new_v4();
    let key = key(scene, 1);
    let mut authority = ResidencyAuthority::default();
    let mut assets = Assets::<Mesh>::default();
    authority.install_scene(scene, 1, Vec::new());
    let resident_mesh = resident_non_animation_payload(&mut authority, &mut assets, key, 1);

    assert!(authority.remove_reason(key, ResidencyReason::Selected, 1));
    assert!(authority.request_reason(
        key,
        ResidencyReason::AnimationRequired,
        1,
        8,
        8,
    ));
    assert!(authority.requested_mask(&key).animation);
    assert_eq!(authority.queue_len(), 1);
    let supplement = authority.begin_next_load().expect("animation supplement");
    assert!(!authority.in_flight_mask(&key).geometry);
    assert!(authority.in_flight_mask(&key).animation);
    assert_eq!(authority.render_asset_id(&key), Some(resident_mesh));

    assert!(authority.complete_cached_payloads(
        &supplement,
        PayloadLoadMask {
            animation: true,
            ..PayloadLoadMask::default()
        },
    ));
    assert!(authority.satisfied_mask(&key).animation);
    assert!(authority.requested_mask(&key).animation);
    assert_eq!(authority.render_asset_id(&key), Some(resident_mesh));
    assert_eq!(authority.queue_len(), 0);
}

#[test]
fn demand_expansion_while_geometry_loads_follows_up_with_animation() {
    let scene = SceneId::new_v4();
    let key = key(scene, 2);
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene, 2, Vec::new());
    assert!(authority.request_reason(key, ResidencyReason::Selected, 2, 8, 8));
    let geometry = authority.begin_next_load().expect("geometry load");

    assert!(authority.request_reason(
        key,
        ResidencyReason::AnimationRequired,
        2,
        8,
        8,
    ));
    assert_eq!(authority.queue_len(), 0);
    assert!(authority.complete_cpu(key, 2, 8, 8));
    let supplement = authority
        .begin_next_load()
        .expect("follow-up animation load");
    assert_eq!(supplement.generation, geometry.generation);
    assert!(authority.in_flight_mask(&key).animation);
}

#[test]
fn animation_already_loading_reconciles_without_duplicate_supplement() {
    let scene = SceneId::new_v4();
    let key = key(scene, 4);
    let mut authority = ResidencyAuthority::default();
    let mut assets = Assets::<Mesh>::default();
    authority.install_scene(scene, 4, Vec::new());
    let resident_mesh = resident_non_animation_payload(&mut authority, &mut assets, key, 4);

    assert!(authority.request_reason(
        key,
        ResidencyReason::AnimationRequired,
        4,
        8,
        8,
    ));
    let animation = authority.begin_next_load().expect("animation load");
    assert!(authority.in_flight_mask(&key).animation);
    assert_eq!(authority.queue_len(), 0);

    assert!(authority.request_reason(
        key,
        ResidencyReason::ActiveViewpoint,
        4,
        8,
        8,
    ));
    assert_eq!(authority.queue_len(), 0);
    assert!(authority.complete_cached_payloads(
        &animation,
        PayloadLoadMask {
            animation: true,
            ..PayloadLoadMask::default()
        },
    ));
    assert!(authority.satisfied_mask(&key).contains(PayloadLoadMask {
        material: true,
        animation: true,
        ..PayloadLoadMask::default()
    }));
    assert_eq!(authority.render_asset_id(&key), Some(resident_mesh));
    assert_eq!(authority.queue_len(), 0);
}

#[test]
fn stale_supplemental_completion_is_rejected_after_generation_replacement() {
    let scene = SceneId::new_v4();
    let key = key(scene, 3);
    let mut authority = ResidencyAuthority::default();
    let mut assets = Assets::<Mesh>::default();
    authority.install_scene(scene, 3, Vec::new());
    let _ = resident_non_animation_payload(&mut authority, &mut assets, key, 3);
    assert!(authority.remove_reason(key, ResidencyReason::Selected, 3));
    assert!(authority.request_reason(key, ResidencyReason::AnimationRequired, 3, 8, 8));
    let supplement = authority.begin_next_load().expect("supplemental load");

    authority.install_scene(scene, 4, Vec::new());
    assert!(!authority.complete_cached_payloads(
        &supplement,
        PayloadLoadMask {
            animation: true,
            ..PayloadLoadMask::default()
        },
    ));
    assert_eq!(authority.satisfied_mask(&key), PayloadLoadMask::default());
    assert_eq!(authority.queue_len(), 0);
}
