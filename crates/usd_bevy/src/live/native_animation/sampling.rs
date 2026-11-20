use bevy::mesh::morph::MeshMorphWeights;
use bevy::prelude::*;
use openusd::usd::{Stage, TimeCode};

use super::AnimationRuntime;

pub(super) fn sample(world: &mut World, stage: &Stage, time: f64) {
    let runtime = world
        .remove_non_send::<AnimationRuntime>()
        .unwrap_or_default();
    for target in &runtime.transforms {
        if let Ok(Some(eval)) =
            crate::read::xform::read_bound_local_xform_at(stage, &target.xform, Some(time))
        {
            crate::route::xform::apply_local_xform_evaluation(world, target.entity, Some(eval));
        }
    }
    for skeleton in &runtime.skeletons {
        let Ok(Some((translations, rotations, scales))) = skeleton
            .animation
            .joint_transform_components(TimeCode::new(time))
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
        if let Ok(Some(weights)) = skeleton
            .animation
            .blend_shape_weights(TimeCode::new(time))
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
    world.insert_non_send(runtime);
}
