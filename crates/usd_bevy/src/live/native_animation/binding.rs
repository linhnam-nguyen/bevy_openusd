use std::collections::HashMap;

use bevy::prelude::*;
use openusd::sdf::{LayerHandle, Path};
use openusd::usd::{SchemaBase, Stage};
use openusd_schemas::skel::{AnimQuery, Animation, Cache, Root, SkinningQuery};

use super::{BlendBinding, SharedPose, SkeletonBinding, TransformBinding};
use crate::live::index::PrimEntities;
use crate::live::path::PathStore;
use crate::live::projection::traverse_predicate;
use crate::route::skel::UsdBlendShapeBinding;

#[derive(Clone)]
pub(super) struct DiscoveredBinding {
    pub(super) prim: String,
    pub(super) _skeleton: Option<Path>,
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
    attribute.might_be_time_varying().unwrap_or(false)
        || attribute.num_time_samples().is_ok_and(|count| count != 0)
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
                    _skeleton: Some(skel_path.clone()),
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
            .and_then(|prim| openusd_schemas::geom::XformQuery::for_prim(&prim).ok())
            .and_then(|q| q.transform_might_be_time_varying().ok())
            .unwrap_or(false)
            || stage
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
                            && (attribute.might_be_time_varying().unwrap_or(false)
                                || attribute.num_time_samples().is_ok_and(|count| count != 0))
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

// The ordered spec stacks include stronger overrides, selected variant specs,
// and cumulative reference layer offsets. Layer handles are local to this stage.
type SpecKey = (LayerHandle, Path, [u64; 2]);
type PoseSource = (Vec<SpecKey>, Vec<SpecKey>);

fn spec_stack(stage: &Stage, path: Path) -> Option<Vec<SpecKey>> {
    let stack = stage.prim(path).ok()?.prim_stack().ok()?;
    if stack.is_empty() {
        return None;
    }
    Some(
        stack
            .into_iter()
            .map(|site| {
                let offset = [site.offset.offset.to_bits(), site.offset.scale.to_bits()];
                (site.layer, site.path, offset)
            })
            .collect(),
    )
}

pub(super) fn collect_skeleton_runtime(world: &mut World, stage: &Stage) -> Vec<SharedPose> {
    let mut poses: Vec<SharedPose> = Vec::new();
    let mut groups: HashMap<PoseSource, usize> = HashMap::new();
    let mut retained_bytes = 0usize;
    let mut mesh_query = world.query::<(Entity, &UsdBlendShapeBinding)>();
    let mut query = world.query::<(
        Option<&crate::prim_ref::UsdPrimRef>,
        &crate::route::skel::UsdSkelAnimDriver,
    )>();
    let mut meshes = HashMap::<&str, Vec<_>>::new();
    for (entity, binding) in mesh_query.iter(world) {
        meshes
            .entry(&binding.animation_source_path)
            .or_default()
            .push((entity, binding));
    }
    for (prim, driver) in query.iter(world) {
        let Ok(source) = openusd::sdf::path(&driver.animation_source_path) else {
            continue;
        };
        let key = prim
            .and_then(|prim| openusd::sdf::path(&prim.path).ok())
            .and_then(|path| Some((spec_stack(stage, path)?, spec_stack(stage, source.clone())?)));
        let index = if let Some(&index) = key.as_ref().and_then(|key| groups.get(key)) {
            index
        } else {
            let Ok(Some(anim_prim)) = Animation::get(stage, source) else {
                continue;
            };
            let Ok(animation) = AnimQuery::new(anim_prim) else {
                continue;
            };
            let index = poses.len();
            poses.push(SharedPose {
                animation,
                skeletons: Vec::new(),
                blend_bindings: Vec::new(),
            });
            if let Some(key) = key {
                let bytes = key
                    .0
                    .iter()
                    .chain(&key.1)
                    .fold(0usize, |bytes, (_, path, _)| {
                        bytes
                            .saturating_add(size_of::<SpecKey>())
                            .saturating_add(path.as_str().len())
                    });
                let next = retained_bytes.saturating_add(bytes);
                if groups.len() < 65_536 && next <= 8 * 1024 * 1024 {
                    groups.insert(key, index);
                    retained_bytes = next;
                }
            }
            index
        };
        let pose = &mut poses[index];
        if let Some(meshes) = meshes.remove(driver.animation_source_path.as_str()) {
            let source_indices = pose
                .animation
                .blend_shape_order()
                .iter()
                .enumerate()
                .map(|(index, name)| (name.as_str(), index))
                .collect::<HashMap<_, _>>();
            for (entity, binding) in meshes {
                pose.blend_bindings.push(BlendBinding {
                    entity,
                    source_indices: binding
                        .names
                        .iter()
                        .map(|name| source_indices.get(name.as_str()).copied())
                        .collect(),
                });
            }
        }
        pose.skeletons.push(SkeletonBinding {
            joints: driver.joint_entities.clone(),
            has_translations: driver.has_translations,
            has_rotations: driver.has_rotations,
            has_scales: driver.has_scales,
        });
    }
    poses
}

#[cfg(test)]
#[path = "../../../tests/unit/live/pose_sharing.rs"]
mod tests;
