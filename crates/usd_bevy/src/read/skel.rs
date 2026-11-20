//! UsdSkel → CPU skinning.
//!
//! For a live viewer, CPU skinning through openusd's resolvers is the robust
//! choice: openusd owns the (subtle) skinning math — joint order remap, geom
//! bind transform, influence counts — and we just push the deformed points
//! back into the Bevy mesh at the current time. This sidesteps reconstructing
//! Bevy GPU skinning (inverse bind poses, joint entities, geom-bind) by hand,
//! and plugs straight into the [`StageTime`](crate::route::StageTime) loop.
//!
//! Limitation: the SkelAnimation joint order is assumed to match the
//! skeleton's; a mismatch falls back to the rest pose rather than deform
//! incorrectly (proper `AnimMapper` remap is a follow-up).

use openusd::gf;
use openusd::sdf::Path;
use openusd::usd::{Attribute, SchemaBase, Stage, TimeCode};
use openusd_schemas::skel::skinning::{
    BlendShapeWeighted, InbetweenRef, apply_blend_shapes, joint_local_to_skel_space,
    resolve_blend_shape_offsets, skinning_transforms,
};
use openusd_schemas::skel::{
    AnimQuery, Animation, BindingAPI as SkelBindingAPI, BindingProperties, BlendShape,
    BlendShapeSchema, Root, Skeleton, SkeletonQuery, SkinningQuery,
};

use super::util::{read_float_vec, read_int_vec, read_rel_first_target};

/// Static blend-shape payload copied into Bevy morph targets during
/// projection. Playback changes only `MeshMorphWeights`.
#[derive(Debug, Clone)]
pub(crate) struct BlendShapeData {
    pub(crate) names: Vec<String>,
    pub(crate) offsets: Vec<Vec<[f32; 3]>>,
    pub(crate) normal_offsets: Vec<Vec<[f32; 3]>>,
    pub(crate) point_indices: Vec<Vec<i32>>,
}

/// Whether `prim` carries skinning influences (i.e. is a skinned mesh).
pub fn is_skinned(stage: &Stage, prim: &Path) -> bool {
    read_float_vec(stage, prim, "primvars:skel:jointWeights")
        .map(|w| !w.is_empty())
        .unwrap_or(false)
}

/// Whether `prim` binds any blend shapes (`skel:blendShapes`).
pub fn has_blend_shapes(stage: &Stage, prim: &Path) -> bool {
    SkelBindingAPI::get(stage, prim.clone())
        .ok()
        .flatten()
        .and_then(|b| b.blend_shapes().ok().flatten())
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// Whether the mesh's bound blend-shape *weights* may vary over time (so the
/// mesh must be re-morphed when [`StageTime`](crate::route::StageTime) moves).
pub fn blend_is_time_varying(stage: &Stage, mesh_path: &Path) -> bool {
    let Some(binding) = binding_of(stage, mesh_path) else {
        return false;
    };
    let Some(skel_path) = binding.skeleton.clone() else {
        return false;
    };
    let Some(anim_path) = animation_source(stage, &skel_path, &binding) else {
        return false;
    };
    matches!(
        Animation::get(stage, anim_path).ok().flatten().and_then(|a| AnimQuery::new(a).ok()),
        Some(anim) if anim.blend_shape_weights_might_be_time_varying().unwrap_or(false)
    )
}

/// Apply the mesh's bound blend shapes to `rest` at `time`, per the canonical
/// UsdSkel pipeline (morph *before* skinning). Returns `None` when the mesh has
/// no effective (non-zero-weight) blend shapes, so the caller keeps `rest`.
///
/// Weights come from the bound `SkelAnimation`, mapped to the mesh's blend
/// shapes *by name* (the animation's `blendShapes` order need not match the
/// mesh's). Inbetween shapes are resolved via [`resolve_blend_shape_offsets`].
fn blend_shape_deform(
    stage: &Stage,
    mesh_path: &Path,
    rest: &[[f32; 3]],
    time: Option<f64>,
) -> Option<Vec<[f32; 3]>> {
    let api = SkelBindingAPI::get(stage, mesh_path.clone()).ok()??;
    let names = api
        .blend_shapes_attr()
        .cast::<Vec<String>>()
        .ok()
        .flatten()?;
    let targets = api.blend_shape_targets().ok()?;
    if names.is_empty() {
        return None;
    }

    // Resolve the weight vector from the bound animation (by-name mapping).
    let binding = binding_of(stage, mesh_path)?;
    let skel_path = binding.skeleton.clone()?;
    let anim_path = animation_source(stage, &skel_path, &binding)?;
    let anim_prim = Animation::get(stage, anim_path).ok()??;
    let anim = AnimQuery::new(anim_prim).ok()?;
    let order = anim.blend_shape_order();
    let weights = anim
        .blend_shape_weights(TimeCode::new(time.unwrap_or(0.0)))
        .ok()??;
    let weight_of = |name: &str| -> f32 {
        order
            .iter()
            .position(|n| n == name)
            .and_then(|i| weights.get(i).copied())
            .unwrap_or(0.0)
    };

    // Collect owned per-shape offset/index data so the `BlendShapeWeighted`
    // slices below can borrow it.
    let mut owned: Vec<(f32, Vec<gf::Vec3f>, Vec<i32>)> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let w = weight_of(name);
        if w == 0.0 {
            continue;
        }
        let Some(target) = targets.get(i) else {
            continue;
        };
        let Ok(Some(bs)) = BlendShape::get(stage, target.clone()) else {
            continue;
        };
        let primary = bs.offsets().ok().flatten().unwrap_or_default();
        let point_indices = bs.point_indices().ok().flatten().unwrap_or_default();
        let inbetweens = bs.inbetweens().unwrap_or_default();
        if inbetweens.is_empty() {
            owned.push((w, primary, point_indices));
        } else {
            // Resolve inbetweens → effective offsets at `w`; apply at weight 1.
            let ib: Vec<(f32, Vec<gf::Vec3f>)> = inbetweens
                .iter()
                .filter_map(|ib| Some((ib.weight().ok()??, ib.offsets().ok()?.unwrap_or_default())))
                .collect();
            let ib_refs: Vec<InbetweenRef> = ib.iter().map(|(w, o)| (*w, o.as_slice())).collect();
            let Some(resolved) = resolve_blend_shape_offsets(w, &ib_refs, &primary) else {
                continue;
            };
            owned.push((1.0, resolved, point_indices));
        }
    }
    if owned.is_empty() {
        return None;
    }

    let shapes: Vec<BlendShapeWeighted> = owned
        .iter()
        .map(|(w, offsets, point_indices)| BlendShapeWeighted {
            weight: *w,
            offsets,
            point_indices,
        })
        .collect();
    let pts: Vec<gf::Vec3f> = rest.iter().map(|p| gf::Vec3f::from(*p)).collect();
    let morphed = apply_blend_shapes(&pts, &shapes)?;
    Some(morphed.into_iter().map(|v| [v.x, v.y, v.z]).collect())
}

/// The blend-shape-morphed points for `mesh_path` at `time`, for a mesh that has
/// blend shapes but no skinning. `None` if there's nothing to morph.
pub fn blend_shaped_points_at(
    stage: &Stage,
    mesh_path: &Path,
    time: Option<f64>,
) -> anyhow::Result<Option<Vec<[f32; 3]>>> {
    let Some(mesh) = super::geom::read_mesh(stage, mesh_path)? else {
        return Ok(None);
    };
    Ok(blend_shape_deform(stage, mesh_path, &mesh.points, time))
}

/// Walk `prim` and its ancestors for the first authored first-target of `rel`.
fn inherited_rel(stage: &Stage, prim: &Path, rel: &str) -> Option<String> {
    let mut cur = Some(prim.clone());
    while let Some(p) = cur {
        if let Ok(Some(t)) = read_rel_first_target(stage, &p, rel) {
            return Some(t);
        }
        cur = p.parent();
    }
    None
}

/// A mesh's `SkelBindingAPI` with the skeleton and animation targets it inherits.
pub(crate) struct MeshBinding {
    binding: SkelBindingAPI,
    skeleton: Option<Path>,
    animation_source: Option<Path>,
}

/// Resolve `skel:animationSource` authored directly on `skel_path` or inherited
/// from an ancestor (for fixtures/assets that author `rel skel:animationSource`
/// on `SkelRoot` without `apiSchemas = ["SkelBindingAPI"]`).
pub(crate) fn inherited_animation_source(stage: &Stage, skel_path: &Path) -> Option<Path> {
    inherited_rel(stage, skel_path, "skel:animationSource")
        .and_then(|s| openusd::sdf::path(&s).ok())
}

/// Resolve the SkelAnimation bound to the mesh: `skel:animationSource` is
/// authored on the *skeleton* (or inherited to it), not necessarily in the
/// mesh's own namespace — so resolve from the skeleton first, then fall back to
/// the mesh's binding.
fn animation_source(stage: &Stage, skel_path: &Path, mesh_binding: &MeshBinding) -> Option<Path> {
    inherited_animation_source(stage, skel_path).or_else(|| mesh_binding.animation_source.clone())
}

/// Resolve a mesh's binding and inherited skeleton / animation targets; only a
/// mesh inside a `SkelRoot` is bound.
pub(crate) fn binding_of(stage: &Stage, mesh_path: &Path) -> Option<MeshBinding> {
    let binding = SkelBindingAPI::get(stage, mesh_path.clone()).ok()??;
    Root::find(binding.prim()).ok()??;
    let target = |name| {
        inherited_rel(stage, mesh_path, name).and_then(|path| openusd::sdf::path(&path).ok())
    };
    Some(MeshBinding {
        skeleton: target("skel:skeleton"),
        animation_source: target("skel:animationSource"),
        binding,
    })
}

/// Resolve the typed animation source for one skinned mesh without exposing
/// the binding-discovery implementation to application cache adapters.
pub fn animation_query_for_mesh(stage: &Stage, mesh_path: &Path) -> Option<AnimQuery> {
    let binding = binding_of(stage, mesh_path)?;
    let skel_path = binding.skeleton.clone()?;
    let anim_path = animation_source(stage, &skel_path, &binding)?;
    let anim_prim = Animation::get(stage, anim_path).ok()??;
    AnimQuery::new(anim_prim).ok()
}

/// Read primary blend-shape offsets once. Inbetween evaluation remains on the
/// CPU compatibility route; native playback uses these static primary targets.
pub(crate) fn blend_shape_data(stage: &Stage, mesh_path: &Path) -> Option<BlendShapeData> {
    let api = SkelBindingAPI::get(stage, mesh_path.clone()).ok()??;
    let names = api
        .blend_shapes_attr()
        .cast::<Vec<String>>()
        .ok()
        .flatten()?;
    let targets = api.blend_shape_targets().ok()?;
    let mut data = BlendShapeData {
        names: Vec::new(),
        offsets: Vec::new(),
        normal_offsets: Vec::new(),
        point_indices: Vec::new(),
    };
    for (index, name) in names.iter().enumerate() {
        let Some(target) = targets.get(index) else {
            continue;
        };
        let Ok(Some(shape)) = BlendShape::get(stage, target.clone()) else {
            continue;
        };
        data.names.push(name.clone());
        data.offsets.push(
            shape
                .offsets()
                .ok()
                .flatten()
                .unwrap_or_default()
                .into_iter()
                .map(|v| [v.x, v.y, v.z])
                .collect(),
        );
        data.normal_offsets.push(
            shape
                .normal_offsets()
                .ok()
                .flatten()
                .unwrap_or_default()
                .into_iter()
                .map(|v| [v.x, v.y, v.z])
                .collect(),
        );
        data.point_indices
            .push(shape.point_indices().ok().flatten().unwrap_or_default());
    }
    (!data.names.is_empty()).then_some(data)
}

/// Whether the skinned mesh's bound SkelAnimation may vary over time (so the
/// mesh must be resampled when [`StageTime`](crate::route::StageTime) moves).
pub fn skin_is_time_varying(stage: &Stage, mesh_path: &Path) -> bool {
    let Some(binding) = binding_of(stage, mesh_path) else {
        return false;
    };
    let Some(skel_path) = binding.skeleton.clone() else {
        return false;
    };
    let Some(anim_path) = animation_source(stage, &skel_path, &binding) else {
        return false;
    };
    matches!(
        Animation::get(stage, anim_path).ok().flatten().and_then(|a| AnimQuery::new(a).ok()),
        Some(anim) if anim.joint_transforms_might_be_time_varying().unwrap_or(false)
    )
}

/// The skinned mesh points for `mesh_path` at `time` (`None` = rest/default),
/// or `None` if the prim isn't a resolvable skinned mesh. The result is in the
/// mesh's local space — a drop-in replacement for its rest `points`.
pub fn skinned_points_at(
    stage: &Stage,
    mesh_path: &Path,
    time: Option<f64>,
) -> anyhow::Result<Option<Vec<[f32; 3]>>> {
    let Some(binding) = binding_of(stage, mesh_path) else {
        return Ok(None);
    };

    // Fast path: skip prims whose `jointIndices`/`jointWeights` are absent or
    // disagree in length (some assets, e.g. Hummingbird, author them
    // inconsistently). openusd now *returns an error* for this rather than
    // panicking, but bailing here avoids building the resolver at all and keeps
    // the log quiet in the common case.
    let n_indices = read_int_vec(stage, mesh_path, "primvars:skel:jointIndices")
        .map(|v| v.len())
        .unwrap_or(0);
    let n_weights = read_float_vec(stage, mesh_path, "primvars:skel:jointWeights")
        .map(|v| v.len())
        .unwrap_or(0);
    if n_indices == 0 || n_indices != n_weights {
        if n_indices != n_weights {
            log::warn!(
                "usd_bevy::skel: {}: jointIndices ({n_indices}) / jointWeights ({n_weights}) \
                 length mismatch — showing the mesh un-skinned",
                mesh_path.as_str()
            );
        }
        return Ok(None);
    }

    let Some(skel_path) = binding.skeleton.clone() else {
        return Ok(None);
    };
    let Some(skeleton) = Skeleton::get(stage, skel_path.clone())? else {
        return Ok(None);
    };
    let Some(resolver) = SkeletonQuery::new(&skeleton)? else {
        return Ok(None);
    };
    let authored = |attribute: Attribute| -> anyhow::Result<Option<Attribute>> {
        Ok(attribute.has_authored_value()?.then_some(attribute))
    };
    let api = &binding.binding;
    let properties = BindingProperties {
        joint_indices: authored(api.joint_indices_attr())?,
        joint_weights: authored(api.joint_weights_attr())?,
        skinning_method: authored(api.skinning_method_attr())?,
        geom_bind_transform: authored(api.geom_bind_transform_attr())?,
        joints: authored(api.joints_attr())?,
        ..BindingProperties::default()
    };
    let skinning = SkinningQuery::new(
        api.prim().clone(),
        properties,
        resolver.joint_order().to_vec(),
        Vec::<String>::new(),
    )?;
    if skinning.is_rigidly_deformed() || !skinning.has_joint_influences() {
        return Ok(None);
    }

    // Joint-local transforms at `time`: from the bound SkelAnimation, else the
    // skeleton's rest pose (which yields the undeformed mesh).
    let rest = || -> Vec<gf::Mat4d> {
        resolver
            .rest_local_transforms()
            .map(<[_]>::to_vec)
            .unwrap_or_default()
    };
    let anim_path = animation_source(stage, &skel_path, &binding);
    let locals: Vec<gf::Mat4d> = match anim_path {
        Some(anim_path) => match Animation::get(stage, anim_path)? {
            Some(anim_prim) => {
                let anim = AnimQuery::new(anim_prim)?;
                let tc = TimeCode::new(time.unwrap_or(0.0));
                match anim.joint_local_transforms(tc)? {
                    Some(l) if l.len() == resolver.joint_order().len() => l,
                    _ => rest(),
                }
            }
            None => rest(),
        },
        None => rest(),
    };
    let Some(skel) = joint_local_to_skel_space(&locals, resolver.topology(), None) else {
        return Ok(None);
    };
    let Some(inverse) = resolver.inverse_bind_transforms() else {
        return Ok(None);
    };
    let Some(skel_xforms) = skinning_transforms(&skel, inverse) else {
        return Ok(None);
    };
    let pose = openusd_schemas::skel::Pose::from(skel_xforms);

    let Some(mesh) = super::geom::read_mesh(stage, mesh_path)? else {
        return Ok(None);
    };
    // Canonical UsdSkel order: morph blend shapes first, then skin the result.
    let rest_pts = blend_shape_deform(stage, mesh_path, &mesh.points, time).unwrap_or(mesh.points);
    let pts: Vec<gf::Vec3f> = rest_pts.iter().map(|p| gf::Vec3f::from(*p)).collect();
    if let Some(skinned) =
        skinning.skinned_points(&pose, &pts, TimeCode::new(time.unwrap_or(0.0)))?
    {
        Ok(Some(skinned.into_iter().map(|v| [v.x, v.y, v.z]).collect()))
    } else {
        Ok(None)
    }
}
