//! Projection-time animation bindings.
//!
//! This module owns the typed animation boundary introduced by M8-OR3-C1.
//! USD paths, skeleton joint entities, inverse bind poses, and morph target
//! names are resolved while a stage is projected. StageTime sampling then
//! updates only existing Bevy components.

use std::collections::HashMap;

use bevy::ecs::hierarchy::ChildOf;
use bevy::mesh::Mesh3d;
use bevy::mesh::morph::{MeshMorphWeights, MorphAttributes};
use bevy::prelude::*;
use openusd::schemas::skel::{
    SkelAnimQuery, SkelBinding, SkelBindingAPI, Skeleton, SkeletonResolver, discover_bindings,
};
use openusd::sdf::Path;
use openusd::usd::{Stage, TimeCode};

use super::index::PrimEntities;
use super::projection::traverse_predicate;
use super::stage::LiveStage;
use crate::route::skel::{UsdBlendShapeBinding, UsdJoint, UsdSkelAnimDriver};

#[derive(Clone)]
struct TransformBinding {
    entity: Entity,
    path: Path,
}

#[derive(Clone)]
struct BlendBinding {
    entity: Entity,
    names: Vec<String>,
}

#[derive(Clone)]
struct SkeletonBinding {
    source: Path,
    joints: Vec<Entity>,
    has_translations: bool,
    has_rotations: bool,
    has_scales: bool,
    blend_bindings: Vec<BlendBinding>,
}

/// Compact, typed playback targets. It is rebuilt only after projection or a
/// structural reconcile, never on every StageTime change.
#[derive(Resource, Default)]
pub(super) struct AnimationRuntime {
    transforms: Vec<TransformBinding>,
    skeletons: Vec<SkeletonBinding>,
}

fn matrix_to_transform(matrix: openusd::gf::Matrix4d) -> Transform {
    let values = std::array::from_fn(|index| matrix.0[index] as f32);
    let (scale, rotation, translation) =
        Mat4::from_cols_array(&values).to_scale_rotation_translation();
    Transform {
        translation,
        rotation,
        scale,
    }
}

fn joint_index(joints: &[String], name: &str) -> Option<usize> {
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

fn selected_joint_indices(joints: &[String], subset: &[String]) -> Option<Vec<usize>> {
    if subset.is_empty() {
        return Some((0..joints.len()).collect());
    }
    subset
        .iter()
        .map(|name| joint_index(joints, name))
        .collect()
}

fn animation_channel_authored(stage: &Stage, path: &Path, name: &str) -> bool {
    let attribute = stage.prim(path.clone()).attribute(name);
    attribute
        .time_sample_times()
        .map(|times| !times.is_empty())
        .unwrap_or(false)
}

fn collect_bindings(stage: &Stage) -> HashMap<String, Vec<SkelBinding>> {
    let mut roots = Vec::new();
    let _ = stage.traverse(traverse_predicate(), |path: &Path| {
        if stage
            .prim(path.clone())
            .type_name()
            .ok()
            .flatten()
            .as_deref()
            == Some("SkelRoot")
        {
            roots.push(path.clone());
        }
    });
    let mut output = HashMap::new();
    for root in roots {
        let Ok(bindings) = discover_bindings(stage, &root) else {
            continue;
        };
        for binding in bindings {
            let Some(skeleton) = binding.skeleton.as_ref() else {
                continue;
            };
            output
                .entry(skeleton.as_str().to_owned())
                .or_insert_with(Vec::new)
                .push(binding);
        }
    }
    output
}

fn blend_targets(
    mesh: &Path,
    data: &crate::read::skel::BlendShapeData,
    output_points: &[usize],
) -> (Vec<MorphAttributes>, Vec<String>) {
    let count = data.names.len().min(bevy::mesh::morph::MAX_MORPH_WEIGHTS);
    let mut attributes = Vec::with_capacity(count.saturating_mul(output_points.len()));
    for shape in 0..count {
        let mut sparse = HashMap::new();
        if let Some(indices) = data.point_indices.get(shape) {
            for (offset, point) in indices.iter().enumerate() {
                sparse.entry(*point).or_insert(offset);
            }
        }
        let offsets = data.offsets.get(shape).cloned().unwrap_or_default();
        let normals = data.normal_offsets.get(shape).cloned().unwrap_or_default();
        let sparse_shape = data
            .point_indices
            .get(shape)
            .is_some_and(|indices| !indices.is_empty());
        for &point in output_points {
            let source = if sparse_shape {
                sparse.get(&(point as i32)).copied().unwrap_or(usize::MAX)
            } else {
                point
            };
            attributes.push(MorphAttributes::from([
                Vec3::from_array(offsets.get(source).copied().unwrap_or([0.0; 3])),
                Vec3::from_array(normals.get(source).copied().unwrap_or([0.0; 3])),
                Vec3::ZERO,
            ]));
        }
    }
    let names = data.names[..count].to_vec();
    let _ = mesh;
    (attributes, names)
}

fn clear_bindings(world: &mut World) {
    let joints = {
        let mut query = world.query::<(Entity, &UsdJoint)>();
        query
            .iter(world)
            .map(|(entity, joint)| (entity, joint.path.matches('/').count()))
            .collect::<Vec<_>>()
    };
    for (entity, _) in joints {
        world.despawn(entity);
    }
    let entities = {
        let mut query = world.query_filtered::<Entity, Or<(
            With<UsdSkelAnimDriver>,
            With<bevy::mesh::skinning::SkinnedMesh>,
            With<MeshMorphWeights>,
            With<UsdBlendShapeBinding>,
        )>>();
        query.iter(world).collect::<Vec<_>>()
    };
    for entity in entities {
        if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
            entity_mut.remove::<UsdSkelAnimDriver>();
            entity_mut.remove::<bevy::mesh::skinning::SkinnedMesh>();
            entity_mut.remove::<MeshMorphWeights>();
            entity_mut.remove::<UsdBlendShapeBinding>();
        }
    }
}

fn attach_native_mesh(
    world: &mut World,
    stage: &Stage,
    map: &PrimEntities,
    binding: &SkelBinding,
    joints: &[String],
    joint_entities: &[Option<Entity>],
    resolver: &SkeletonResolver,
    source: &Path,
) {
    let Some(entity) = map.entity(&binding.prim) else {
        return;
    };
    let Ok(mesh_path) = openusd::sdf::path(&binding.prim) else {
        return;
    };
    let Ok(Some(read)) = crate::read::geom::read_mesh(stage, &mesh_path) else {
        return;
    };
    let skinned = crate::read::skel::is_skinned(stage, &mesh_path);
    let blend_data = crate::read::skel::blend_shape_data(stage, &mesh_path);
    if !skinned && blend_data.is_none() {
        return;
    }
    let selected = if skinned {
        let Some(subset) = binding.binding.joint_subset().ok() else {
            return;
        };
        let Some(selected) = selected_joint_indices(joints, &subset) else {
            return;
        };
        selected
    } else {
        Vec::new()
    };
    let Some(mut mesh) = (if skinned {
        let Ok(attrs) = crate::mesh::skin_attrs_from_binding(
            &binding.binding,
            read.points.len(),
            selected.len(),
        ) else {
            return;
        };
        Some(crate::mesh::mesh_from_usd_with_skin(&read, &attrs))
    } else {
        Some(crate::mesh::mesh_from_usd(&read))
    }) else {
        return;
    };
    let output_points = crate::mesh::output_point_indices(&read);
    let blend_names = blend_data
        .as_ref()
        .map(|data| {
            let (attributes, names) = blend_targets(&mesh_path, data, &output_points);
            mesh.set_morph_targets(attributes);
            mesh.set_morph_target_names(names.clone());
            names
        })
        .unwrap_or_default();
    let Some(mesh_handle) = world
        .get_resource_mut::<Assets<Mesh>>()
        .map(|mut assets| assets.add(mesh))
    else {
        return;
    };
    let inverse_handle = if skinned {
        let selected_entities = selected
            .iter()
            .map(|&index| joint_entities.get(index).copied().flatten())
            .collect::<Option<Vec<_>>>();
        let Some(selected_entities) = selected_entities else {
            return;
        };
        let inverse = selected
            .iter()
            .map(|&index| {
                resolver
                    .inverse_bind_transforms()
                    .get(index)
                    .copied()
                    .map(|value| Mat4::from_cols_array(&std::array::from_fn(|i| value.0[i] as f32)))
                    .unwrap_or(Mat4::IDENTITY)
            })
            .collect::<Vec<_>>();
        let handle = world
            .resource_mut::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>()
            .add(bevy::mesh::skinning::SkinnedMeshInverseBindposes::from(
                inverse,
            ));
        if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
            entity_mut.insert(bevy::mesh::skinning::SkinnedMesh {
                inverse_bindposes: handle.clone(),
                joints: selected_entities,
            });
        }
        Some(handle)
    } else {
        None
    };
    if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
        entity_mut.insert(Mesh3d(mesh_handle));
        if !blend_names.is_empty() {
            entity_mut.insert((
                UsdBlendShapeBinding {
                    names: blend_names.clone(),
                    animation_source_path: source.as_str().to_owned(),
                },
                MeshMorphWeights::Value {
                    weights: vec![0.0; blend_names.len()],
                },
            ));
        }
        let _ = inverse_handle;
    }
}

/// Rebuild all native animation components after a projection or structural
/// reconcile. Existing joint entities are deliberately replaced so stale
/// skeleton topology cannot survive a USD resync.
pub(super) fn rebuild(world: &mut World, live: &LiveStage, map: &PrimEntities) {
    clear_bindings(world);
    let stage = &live.stage;
    let by_skeleton = collect_bindings(stage);
    let mut skeleton_paths = by_skeleton.keys().cloned().collect::<Vec<_>>();
    skeleton_paths.sort();
    for skeleton_path in skeleton_paths {
        let Ok(path) = openusd::sdf::path(&skeleton_path) else {
            continue;
        };
        let Some(skeleton_entity) = map.entity(&skeleton_path) else {
            continue;
        };
        let Ok(Some(skeleton)) = Skeleton::get(stage, path.clone()) else {
            continue;
        };
        let Ok(joints) = skeleton.joints() else {
            continue;
        };
        let Ok(resolver) = SkeletonResolver::from_skeleton(&skeleton) else {
            continue;
        };
        let parents = skeleton
            .joint_parent_indices()
            .unwrap_or_else(|_| vec![None; joints.len()]);
        let mut entities = vec![None; joints.len()];
        for index in 0..joints.len() {
            let parent = parents
                .get(index)
                .and_then(|parent| *parent)
                .and_then(|parent| entities.get(parent).copied().flatten())
                .unwrap_or(skeleton_entity);
            let local = resolver
                .rest_pose_local()
                .get(index)
                .copied()
                .unwrap_or(openusd::gf::Matrix4d::IDENTITY);
            let name = joints[index]
                .rsplit_once('/')
                .map(|(_, leaf)| leaf)
                .unwrap_or(&joints[index]);
            let entity = world
                .spawn((
                    Name::new(name.to_owned()),
                    matrix_to_transform(local),
                    Visibility::default(),
                    UsdJoint {
                        skeleton_path: skeleton_path.clone(),
                        path: joints[index].clone(),
                        index: index as u32,
                    },
                    ChildOf(parent),
                ))
                .id();
            entities[index] = Some(entity);
        }
        let bindings = by_skeleton.get(&skeleton_path).cloned().unwrap_or_default();
        let source = bindings
            .iter()
            .find_map(|binding| binding.animation_source.clone())
            .or_else(|| {
                SkelBindingAPI::get(stage, path.clone())
                    .ok()
                    .flatten()
                    .and_then(|api| api.inherited_animation_source().ok().flatten())
            });
        let Some(source) = source else { continue };
        let Ok(Some(animation)) = SkelAnimQuery::new(stage, source.clone()) else {
            continue;
        };
        let animation_joints = animation.joint_order();
        let mapped = animation_joints
            .iter()
            .map(|name| joint_index(&joints, name).and_then(|index| entities[index]))
            .collect::<Vec<_>>();
        let driver = UsdSkelAnimDriver {
            anim_name: source
                .as_str()
                .rsplit_once('/')
                .map(|(_, leaf)| leaf.to_owned())
                .unwrap_or_else(|| source.as_str().to_owned()),
            animation_source_path: source.as_str().to_owned(),
            skeleton_joints: joints.clone(),
            skeleton_joint_entities: entities.clone(),
            joint_entities: mapped,
            has_translations: animation_channel_authored(stage, &source, "translations"),
            has_rotations: animation_channel_authored(stage, &source, "rotations"),
            has_scales: animation_channel_authored(stage, &source, "scales"),
            has_blend_shape_weights: animation_channel_authored(
                stage,
                &source,
                "blendShapeWeights",
            ),
            blend_shape_names: animation.blend_shape_order().to_vec(),
            blend_shape_weights: Vec::new(),
        };
        world.entity_mut(skeleton_entity).insert(driver.clone());
        for binding in bindings {
            attach_native_mesh(
                world, stage, map, &binding, &joints, &entities, &resolver, &source,
            );
        }
    }
    let mut transforms = Vec::new();
    for (path, entity) in map.iter() {
        let Ok(path) = openusd::sdf::path(path) else {
            continue;
        };
        let animated = stage
            .prim(path.clone())
            .attributes()
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
            transforms.push(TransformBinding { entity, path });
        }
    }
    let mut skeletons = Vec::new();
    let mut query = world.query::<&UsdSkelAnimDriver>();
    let drivers = query.iter(world).cloned().collect::<Vec<_>>();
    for driver in drivers {
        let Ok(source) = openusd::sdf::path(&driver.animation_source_path) else {
            continue;
        };
        let mut blend_bindings = Vec::new();
        let mut mesh_query = world.query::<(Entity, &UsdBlendShapeBinding)>();
        for (entity, binding) in mesh_query.iter(world) {
            if binding.animation_source_path == driver.animation_source_path {
                blend_bindings.push(BlendBinding {
                    entity,
                    names: binding.names.clone(),
                });
            }
        }
        skeletons.push(SkeletonBinding {
            source,
            joints: driver.joint_entities.iter().copied().flatten().collect(),
            has_translations: driver.has_translations,
            has_rotations: driver.has_rotations,
            has_scales: driver.has_scales,
            blend_bindings,
        });
    }
    world.insert_resource(AnimationRuntime {
        transforms,
        skeletons,
    });
}

/// Sample pre-bound native targets. No route registry, USD path parsing, mesh
/// reads, or mesh/material asset allocation occurs here.
pub(super) fn sample(world: &mut World, stage: &Stage, time: f64) {
    let runtime = world
        .remove_resource::<AnimationRuntime>()
        .unwrap_or_default();
    for target in &runtime.transforms {
        if let Ok(Some(value)) =
            crate::read::xform::read_transform_at(stage, &target.path, Some(time))
            && let Some(mut transform) = world.get_mut::<Transform>(target.entity)
        {
            *transform = crate::live::projection::to_bevy_transform(value);
        }
    }
    for skeleton in &runtime.skeletons {
        let Ok(Some(animation)) = SkelAnimQuery::new(stage, skeleton.source.clone()) else {
            continue;
        };
        let Ok((translations, rotations, scales)) =
            animation.compute_joint_local_transform_components(stage, TimeCode::new(time))
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
        if let Ok(weights) = animation.compute_blend_shape_weights(stage, TimeCode::new(time)) {
            let names = animation.blend_shape_order();
            for binding in &skeleton.blend_bindings {
                let Some(mut morph) = world.get_mut::<MeshMorphWeights>(binding.entity) else {
                    continue;
                };
                let MeshMorphWeights::Value { weights: output } = &mut *morph else {
                    continue;
                };
                for (index, name) in binding.names.iter().enumerate() {
                    if let Some(weight) = output.get_mut(index) {
                        *weight = names
                            .iter()
                            .position(|source| source == name)
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
