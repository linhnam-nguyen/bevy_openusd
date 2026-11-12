use super::*;

#[test]
fn scene_animation_payload_covers_bound_skeleton_data() -> Result<()> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/stages/skel.usda");
    let stage = Stage::open(path)?;
    let bytes = prepare_animation_payload(&stage, "/World/Rig/Arm")?
        .expect("bound mesh has a Scene animation payload");
    let payload: SceneAnimationBlob = serde_json::from_slice(&bytes)?;
    payload.validate()?;
    assert_eq!(payload.version, SCENE_ANIMATION_VERSION);
    assert_eq!(payload.source_path, "/World/Rig/Anim");
    assert_eq!(payload.joint_order, vec!["root", "root/tip"]);
    assert_eq!(payload.samples.len(), 1);
    assert_eq!(payload.samples[0].translations.len(), 2);
    assert_eq!(payload.samples[0].rotations.len(), 2);
    assert_eq!(payload.samples[0].scales.len(), 2);
    Ok(())
}
