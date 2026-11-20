use std::collections::HashMap;

use bevy::prelude::*;
use openusd::sdf::Path;
use openusd::usd::{SchemaBase, Stage};
use openusd_schemas::skel::{AnimQuery, Animation, Cache, Root, SkinningQuery};

use super::{BlendBinding, SkeletonBinding, TransformBinding};
use crate::live::index::PrimEntities;
use crate::live::path::PathStore;
use crate::live::projection::traverse_predicate;
use crate::route::skel::UsdBlendShapeBinding;

#[derive(Clone)]
pub(super) struct DiscoveredBinding {
    pub(super) prim: String,
    pub(super) skeleton: Option<Path>,
    pub(super) animation_source: Option<Path>,
    pub(super) skinning: SkinningQuery,
}

pub(super) fn matrix_to_transform(matrix: openusd::gf::Mat4d) -> Transform {
    let values = std::array::from_fn(|index| matrix.0[index] as f32);
    let (scale, rotation, translation) =
        Mat4::from_cols_array(&values).to_scale_rotation_translation();
    Transform {
        translation,
        rotation,
        scale,
    }
}

pub(super) fn joint_index(joints: &[String], name: &str) -> Option<usize> {
    joints.iter().position(|joint| joint == name).or_else(|| {
        let leaf = name
            .rsplit_once('/')
            .map(|(_, value)| value)
            .unwrap_or(name);
        joints.iter().position(|joint| {
            joint
                .rsplit_once('/')
                .map(|(_, value)| value)
                .unwrap_or(joint)
                == leaf
        })
    })
}

pub(super) fn selected_joint_indices(joints: &[String], subset: &[String]) -> Option<Vec<usize>> {
    if subset.is_empty() {
        return Some((0..joints.len()).collect());
    }
    subset
        .iter()
        .map(|name| joint_index(joints, name))
        .collect()
}

pub(super) fn animation_channel_authored(stage: &Stage, path: &Path, name: &str) -> bool {
    let Ok(prim) = stage.prim(path.clone()) else {
        return false;
    };
    let attribute = prim.attribute(name);
    attribute
        .time_sample_times()
        .map(|times| !times.is_empty())
        .unwrap_or(false)
}

pub(super) fn collect_bindings(stage: &Stage) -> HashMap<String, Vec<DiscoveredBinding>> {
    let mut roots = Vec::new();
    let _ = stage.traverse(traverse_predicate(), |path: &Path| {
        if stage
            .prim(path.clone())
            .ok()
            .and_then(|p| p.type_name().ok().flatten())
            .as_deref()
            == Some("SkelRoot")
        {
            roots.push(path.clone());
        }
    });
    let mut cache = Cache::new();
    let mut output = HashMap::new();
    for root_path in roots {
        let Ok(Some(root)) = Root::get(stage, root_path) else {
            continue;
        };
        if cache.populate(&root, traverse_predicate()).is_err() {
            continue;
        }
        let Ok(bindings) = cache.skel_bindings(&root, traverse_predicate()) else {
            continue;
        };
        for binding in bindings {
            let skeleton = binding.skeleton();
            let skel_path = skeleton.path().clone();
            let animation_source = skeleton
                .animation_source()
                .ok()
                .flatten()
                .map(|anim| anim.path().clone());
            let entry = output
                .entry(skel_path.as_str().to_owned())
                .or_insert_with(Vec::new);
            for target in binding.skinning_targets() {
                entry.push(DiscoveredBinding {
                    prim: target.prim().path().as_str().to_owned(),
                    skeleton: Some(skel_path.clone()),
                    animation_source: animation_source.clone(),
                    skinning: target.clone(),
                });
            }
        }
    }
    output
}

pub(super) fn collect_transform_bindings(
    world: &World,
    stage: &Stage,
    map: &PrimEntities,
) -> Vec<TransformBinding> {
    let paths = world.resource::<PathStore>();
    let mut transforms = Vec::new();
    for (path, entity) in map.iter(paths) {
        let Ok(path) = openusd::sdf::path(path) else {
            continue;
        };
        let animated = stage
            .prim(path.clone())
            .ok()
            .and_then(|prim| prim.attributes().ok())
            .map(|attrs| {
                attrs.iter().any(|attribute| {
                    attribute
                        .path()
                        .as_str()
                        .rsplit_once('.')
                        .map(|(_, name)| name.starts_with("xformOp"))
                        .unwrap_or(false)
                        && attribute
                            .time_sample_times()
                            .map(|times| !times.is_empty())
                            .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        if animated {
            let Ok(Some(xform)) = crate::read::xform::bind_transform(stage, &path) else {
                continue;
            };
            transforms.push(TransformBinding { entity, xform });
        }
    }
    transforms
}

pub(super) fn collect_skeleton_runtime(world: &mut World, stage: &Stage) -> Vec<SkeletonBinding> {
    let mut skeletons = Vec::new();
    let mut query = world.query::<&crate::route::skel::UsdSkelAnimDriver>();
    let drivers = query.iter(world).cloned().collect::<Vec<_>>();
    for driver in drivers {
        let Ok(source) = openusd::sdf::path(&driver.animation_source_path) else {
            continue;
        };
        let Ok(Some(anim_prim)) = Animation::get(stage, source) else {
            continue;
        };
        let Ok(animation) = AnimQuery::new(anim_prim) else {
            continue;
        };
        let source_indices = animation
            .blend_shape_order()
            .iter()
            .enumerate()
            .map(|(index, name)| (name.as_str(), index))
            .collect::<HashMap<_, _>>();
        let mut blend_bindings = Vec::new();
        let mut mesh_query = world.query::<(Entity, &UsdBlendShapeBinding)>();
        for (entity, binding) in mesh_query.iter(world) {
            if binding.animation_source_path == driver.animation_source_path {
                blend_bindings.push(BlendBinding {
                    entity,
                    source_indices: binding
                        .names
                        .iter()
                        .map(|name| source_indices.get(name.as_str()).copied())
                        .collect(),
                });
            }
        }
        skeletons.push(SkeletonBinding {
            animation,
            joints: driver.joint_entities.iter().copied().flatten().collect(),
            has_translations: driver.has_translations,
            has_rotations: driver.has_rotations,
            has_scales: driver.has_scales,
            blend_bindings,
        });
    }
    skeletons
}
