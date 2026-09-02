use bevy::mesh::morph::MeshMorphWeights;
use bevy::prelude::*;
use openusd::usd::{Stage, TimeCode};

use super::AnimationRuntime;

pub(super) fn sample(world: &mut World, stage: &Stage, time: f64) {
    let runtime = world
        .remove_resource::<AnimationRuntime>()
        .unwrap_or_default();
    for target in &runtime.transforms {
        if let Ok(Some(value)) =
            crate::read::xform::read_bound_transform_at(stage, &target.xform, Some(time))
            && let Some(mut transform) = world.get_mut::<Transform>(target.entity)
        {
            *transform = crate::live::projection::to_bevy_transform(value);
        }
    }
    for skeleton in &runtime.skeletons {
        let Ok((translations, rotations, scales)) = skeleton
            .animation
            .compute_joint_local_transform_components(stage, TimeCode::new(time))
        else {
            continue;
        };
        for (index, entity) in skeleton.joints.iter().enumerate() {
            let Some(mut transform) = world.get_mut::<Transform>(*entity) else {
                continue;
            };
            if skeleton.has_translations
                && let Some(value) = translations.get(index)
            {
                transform.translation = Vec3::new(value.x, value.y, value.z);
            }
            if skeleton.has_rotations
                && let Some(value) = rotations.get(index)
            {
                transform.rotation = Quat::from_xyzw(value.x, value.y, value.z, value.w);
            }
            if skeleton.has_scales
                && let Some(value) = scales.get(index)
            {
                transform.scale = Vec3::new(value.x, value.y, value.z);
            }
        }
        if let Ok(weights) = skeleton
            .animation
            .compute_blend_shape_weights(stage, TimeCode::new(time))
        {
            for binding in &skeleton.blend_bindings {
                let Some(mut morph) = world.get_mut::<MeshMorphWeights>(binding.entity) else {
                    continue;
                };
                let MeshMorphWeights::Value { weights: output } = &mut *morph else {
                    continue;
                };
                for (index, source_index) in binding.source_indices.iter().enumerate() {
                    if let Some(weight) = output.get_mut(index) {
                        *weight = source_index
                            .and_then(|source| weights.get(source))
                            .copied()
                            .unwrap_or_default();
                    }
                }
            }
        }
    }
    world.insert_resource(runtime);
}
