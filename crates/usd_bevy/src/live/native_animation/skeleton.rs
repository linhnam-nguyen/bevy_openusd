use std::collections::HashMap;

use bevy::ecs::hierarchy::ChildOf;
use bevy::mesh::morph::MeshMorphWeights;
use bevy::prelude::*;
use openusd::usd::Stage;
use openusd_schemas::skel::{
    AnimMapper, AnimQuery, Animation, BindingAPI as SkelBindingAPI, Skeleton, SkeletonQuery,
};

use super::binding::{
    DiscoveredBinding, animation_channel_authored, joint_index, matrix_to_transform,
};
use super::mesh_binding::attach_native_mesh;
use crate::live::index::PrimEntities;
use crate::live::path::PathStore;
use crate::route::skel::{UsdBlendShapeBinding, UsdJoint, UsdSkelAnimDriver};

pub(super) fn clear_bindings(world: &mut World) {
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
            With<crate::extended_skin::ExtendedSkinMesh>,
            With<super::CpuDeformBinding>,
        )>>();
        query.iter(world).collect::<Vec<_>>()
    };
    for entity in entities {
        let standard = world
            .get::<MeshMaterial3d<crate::extended_skin::ExtendedSkinMaterial>>(entity)
            .and_then(|material| {
                world
                    .get_resource::<Assets<crate::extended_skin::ExtendedSkinMaterial>>()?
                    .get(&material.0)
            })
            .map(|material| material.base.clone())
            .and_then(|base| {
                world
                    .get_resource_mut::<Assets<StandardMaterial>>()
                    .map(|mut assets| assets.add(base))
            });
        if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
            entity_mut.remove::<UsdSkelAnimDriver>();
            entity_mut.remove::<bevy::mesh::skinning::SkinnedMesh>();
            entity_mut.remove::<bevy::camera::visibility::DynamicSkinnedMeshBounds>();
            entity_mut.remove::<MeshMorphWeights>();
            entity_mut.remove::<UsdBlendShapeBinding>();
            entity_mut.remove::<crate::extended_skin::ExtendedSkinMesh>();
            entity_mut.remove::<MeshMaterial3d<crate::extended_skin::ExtendedSkinMaterial>>();
            entity_mut.remove::<super::CpuDeformBinding>();
            if let Some(standard) = standard {
                entity_mut.insert_if_new(MeshMaterial3d(standard));
            }
        }
    }
}

pub(super) fn rebuild_skeletons(
    world: &mut World,
    stage: &Stage,
    map: &PrimEntities,
    by_skeleton: &HashMap<String, Vec<DiscoveredBinding>>,
) {
    let mut skeleton_paths = by_skeleton.keys().cloned().collect::<Vec<_>>();
    skeleton_paths.sort();
    for skeleton_path in skeleton_paths {
        let Ok(path) = openusd::sdf::path(&skeleton_path) else {
            continue;
        };
        let skeleton_entity = {
            let paths = world.resource::<PathStore>();
            map.entity(paths, &skeleton_path)
        };
        let Some(skeleton_entity) = skeleton_entity else {
            continue;
        };
        let Ok(Some(skeleton)) = Skeleton::get(stage, path.clone()) else {
            continue;
        };
        let Ok(Some(resolver)) = SkeletonQuery::new(&skeleton) else {
            continue;
        };
        let joints = resolver.joint_order().to_vec();
        let parents: Vec<Option<usize>> = resolver
            .topology()
            .parents()
            .iter()
            .map(|&parent| usize::try_from(parent).ok())
            .collect();
        let rest_local = resolver.rest_local_transforms();
        let mut entities = vec![None; joints.len()];
        for index in 0..joints.len() {
            let parent = parents
                .get(index)
                .and_then(|parent| *parent)
                .and_then(|parent| entities.get(parent).copied().flatten())
                .unwrap_or(skeleton_entity);
            let local = rest_local
                .and_then(|rest| rest.get(index).copied())
                .unwrap_or(openusd::gf::Mat4d::IDENTITY);
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
                    .and_then(|api| api.animation_source().ok().flatten())
                    .map(|anim| anim.path().clone())
            })
            .or_else(|| crate::read::skel::inherited_animation_source(stage, &path));
        let Some(source) = source else {
            continue;
        };
        let Ok(Some(anim_prim)) = Animation::get(stage, source.clone()) else {
            continue;
        };
        let Ok(animation) = AnimQuery::new(anim_prim) else {
            continue;
        };
        let animation_joints = animation.joint_order();
        let mapper = AnimMapper::new(&joints, animation_joints);
        let mapped = (0..animation_joints.len())
            .map(|anim_slot| {
                mapper
                    .source_index(anim_slot)
                    .or_else(|| joint_index(&joints, &animation_joints[anim_slot]))
                    .and_then(|index| entities[index])
            })
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
        world.entity_mut(skeleton_entity).insert(driver);
        for binding in bindings {
            attach_native_mesh(
                world, stage, map, &binding, &joints, &entities, &resolver, &source,
            );
        }
    }
}
