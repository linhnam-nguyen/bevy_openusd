use std::collections::HashMap;

use bevy::mesh::Mesh3d;
use bevy::mesh::morph::{MeshMorphWeights, MorphAttributes};
use bevy::prelude::*;
use openusd::sdf::Path;
use openusd::usd::Stage;
use openusd_schemas::skel::{SkeletonQuery, SkinningMethod};

use super::CpuDeformBinding;
use super::binding::{DiscoveredBinding, selected_joint_indices};
use crate::live::index::PrimEntities;
use crate::live::path::PathStore;
use crate::mesh::SkinFidelity;
use crate::route::skel::UsdBlendShapeBinding;

fn blend_targets(
    data: &crate::read::skel::BlendShapeData,
    output_points: &[usize],
) -> (Vec<MorphAttributes>, Vec<String>) {
    let count = data.names.len();
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
    (attributes, names)
}

pub(super) fn attach_native_mesh(
    world: &mut World,
    stage: &Stage,
    map: &PrimEntities,
    binding: &DiscoveredBinding,
    joints: &[String],
    joint_entities: &[Option<Entity>],
    resolver: &SkeletonQuery,
    source: &Path,
) {
    let entity = {
        let paths = world.resource::<PathStore>();
        map.entity(paths, &binding.prim)
    };
    let Some(entity) = entity else {
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
    let fallback = |world: &mut World| {
        world
            .entity_mut(entity)
            .insert(CpuDeformBinding(mesh_path.clone()));
    };
    if skinned
        && (binding.skinning.skinning_method().ok().flatten()
            != Some(SkinningMethod::ClassicLinear)
            || binding.skinning.geom_bind_transform(None).ok()
                != Some(openusd::gf::Mat4d::IDENTITY)
            || binding
                .skinning
                .geom_bind_transform_attr()
                .is_some_and(|attr| attr.num_time_samples().is_ok_and(|count| count != 0))
            || [
                binding.skinning.joint_indices_primvar(),
                binding.skinning.joint_weights_primvar(),
            ]
            .into_iter()
            .flatten()
            .any(|primvar| {
                primvar
                    .attribute()
                    .num_time_samples()
                    .is_ok_and(|count| count != 0)
            }))
    {
        fallback(world);
        return;
    }
    let selected = if skinned {
        let subset = binding.skinning.joint_order().unwrap_or(&[]);
        let Some(selected) = selected_joint_indices(joints, subset) else {
            fallback(world);
            return;
        };
        selected
    } else {
        Vec::new()
    };
    let fidelity = if skinned {
        match crate::mesh::skin_fidelity(&binding.skinning, read.points.len(), selected.len()) {
            Ok(fidelity) => fidelity,
            Err(_) => {
                fallback(world);
                return;
            }
        }
    } else {
        SkinFidelity::Standard4
    };
    let base_material = world
        .get::<MeshMaterial3d<StandardMaterial>>(entity)
        .and_then(|material| {
            world
                .get_resource::<Assets<StandardMaterial>>()?
                .get(&material.0)
        })
        .cloned();
    let extended_supported = base_material.is_some()
        && world.contains_resource::<Assets<crate::extended_skin::ExtendedSkinMaterial>>();
    if blend_data.as_ref().is_some_and(|data| {
        data.requires_cpu || data.names.len() > bevy::mesh::morph::MAX_MORPH_WEIGHTS
    }) || (fidelity == SkinFidelity::Extended16 && !extended_supported)
    {
        fallback(world);
        return;
    }
    let Some(mut mesh) = (if skinned {
        match fidelity {
            crate::mesh::SkinFidelity::Standard4 => {
                let Ok(attrs) = crate::mesh::skin_attrs_from_binding(
                    &binding.skinning,
                    read.points.len(),
                    selected.len(),
                ) else {
                    fallback(world);
                    return;
                };
                Some(crate::mesh::mesh_from_usd_with_skin(&read, &attrs))
            }
            crate::mesh::SkinFidelity::Extended16 => {
                let Ok(attrs) = crate::mesh::extended_skin_attrs_from_binding(
                    &binding.skinning,
                    read.points.len(),
                    selected.len(),
                ) else {
                    fallback(world);
                    return;
                };
                Some(crate::mesh::mesh_from_usd_with_extended_skin(&read, &attrs))
            }
        }
    } else {
        Some(crate::mesh::mesh_from_usd(&read))
    }) else {
        return;
    };
    let output_points = crate::mesh::output_point_indices(&read);
    let blend_names = blend_data
        .as_ref()
        .map(|data| {
            let (attributes, names) = blend_targets(data, &output_points);
            mesh.set_morph_targets(attributes);
            mesh.set_morph_target_names(names.clone());
            names
        })
        .unwrap_or_default();
    let dynamic_bounds = skinned
        && fidelity == SkinFidelity::Standard4
        && blend_names.is_empty()
        && mesh.generate_skinned_mesh_bounds().is_ok();
    let Some(mesh_handle) = world
        .get_resource_mut::<Assets<Mesh>>()
        .map(|mut assets| assets.add(mesh))
    else {
        return;
    };
    if skinned {
        let selected_entities = selected
            .iter()
            .map(|&index| joint_entities.get(index).copied().flatten())
            .collect::<Option<Vec<_>>>();
        let Some(selected_entities) = selected_entities else {
            fallback(world);
            return;
        };
        let Some(inverse) = selected
            .iter()
            .map(|&index| {
                resolver
                    .inverse_bind_transforms()
                    .and_then(|transforms| transforms.get(index).copied())
                    .map(|value| Mat4::from_cols_array(&std::array::from_fn(|i| value.0[i] as f32)))
            })
            .collect::<Option<Vec<_>>>()
        else {
            fallback(world);
            return;
        };
        let Some(mut inverse_assets) =
            world.get_resource_mut::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>()
        else {
            fallback(world);
            return;
        };
        let handle = inverse_assets.add(bevy::mesh::skinning::SkinnedMeshInverseBindposes::from(
            inverse,
        ));
        if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
            entity_mut.insert(bevy::mesh::skinning::SkinnedMesh {
                inverse_bindposes: handle,
                joints: selected_entities,
            });
        }
    }
    if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
        entity_mut.insert(Mesh3d(mesh_handle));
        if dynamic_bounds {
            entity_mut.insert(bevy::camera::visibility::DynamicSkinnedMeshBounds);
        }
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
        if fidelity == crate::mesh::SkinFidelity::Extended16 {
            entity_mut.insert(crate::extended_skin::ExtendedSkinMesh);
        }
    }
    if fidelity == crate::mesh::SkinFidelity::Extended16 {
        if let Some(base) = base_material {
            let _ = crate::extended_skin::set_extended_material(world, entity, base);
        }
    }
}
