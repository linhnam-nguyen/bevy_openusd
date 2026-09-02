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
use openusd::schemas::skel::skinning::{
    BlendShapeWeighted, InbetweenRef, apply_blend_shapes, resolve_blend_shape_offsets,
};
use openusd::schemas::skel::{
    BlendShape, SkelAnimQuery, SkelBinding, SkelBindingAPI, Skeleton, SkeletonResolver,
    SkinningResolver, discover_bindings,
};
use openusd::sdf::Path;
use openusd::usd::{Stage, TimeCode};

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
        .and_then(|b| b.blend_shapes().ok())
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
        SkelAnimQuery::new(stage, anim_path),
        Ok(Some(anim)) if anim.blend_shape_weights_might_be_time_varying()
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
    let names = api.blend_shapes().ok()?;
    let targets = api.blend_shape_targets().ok()?;
    if names.is_empty() {
        return None;
    }

    // Resolve the weight vector from the bound animation (by-name mapping).
    let binding = binding_of(stage, mesh_path)?;
    let skel_path = binding.skeleton.clone()?;
    let anim_path = animation_source(stage, &skel_path, &binding)?;
    let anim = SkelAnimQuery::new(stage, anim_path).ok()??;
    let order = anim.blend_shape_order();
    let weights = anim
        .compute_blend_shape_weights(stage, TimeCode::new(time.unwrap_or(0.0)))
        .ok()?;
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
        let primary = bs.offsets().unwrap_or_default();
        let point_indices = bs.point_indices().unwrap_or_default();
        let inbetweens = bs.inbetweens().unwrap_or_default();
        if inbetweens.is_empty() {
            owned.push((w, primary, point_indices));
        } else {
            // Resolve inbetweens → effective offsets at `w`; apply at weight 1.
            let ib: Vec<(f32, Vec<gf::Vec3f>)> = inbetweens
                .iter()
                .filter_map(|ib| Some((ib.weight?, ib.offsets.clone())))
                .collect();
            let ib_refs: Vec<InbetweenRef> = ib.iter().map(|(w, o)| (*w, o.as_slice())).collect();
            let resolved = resolve_blend_shape_offsets(w, &ib_refs, &primary);
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
    let morphed = apply_blend_shapes(&pts, &shapes);
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

/// Resolve the SkelAnimation bound to the mesh: `skel:animationSource` is
/// authored on the *skeleton* (or inherited to it), not necessarily in the
/// mesh's own namespace — so resolve from the skeleton first, then fall back to
/// the mesh's binding.
fn animation_source(stage: &Stage, skel_path: &Path, mesh_binding: &SkelBinding) -> Option<Path> {
    inherited_rel(stage, skel_path, "skel:animationSource")
        .and_then(|s| openusd::sdf::path(&s).ok())
        .or_else(|| mesh_binding.animation_source.clone())
}

/// The enclosing `SkelRoot` of `prim` (walking up, inclusive), which scopes
/// UsdSkel binding resolution.
fn enclosing_skel_root(stage: &Stage, prim: &Path) -> Option<Path> {
    let mut cur = Some(prim.clone());
    while let Some(p) = cur {
        let is_root =
            stage.prim(p.clone()).type_name().ok().flatten().as_deref() == Some("SkelRoot");
        if is_root {
            return Some(p);
        }
        cur = p.parent();
    }
    None
}

/// Resolve the fully-inherited binding (skeleton + animation source) for the
/// skinned mesh at `mesh_path`, via `discover_bindings` over its SkelRoot.
pub(crate) fn binding_of(stage: &Stage, mesh_path: &Path) -> Option<SkelBinding> {
    let skel_root = enclosing_skel_root(stage, mesh_path)?;
    let bindings = discover_bindings(stage, &skel_root).ok()?;
    bindings.into_iter().find(|b| b.prim == mesh_path.as_str())
}

/// Read primary blend-shape offsets once. Inbetween evaluation remains on the
/// CPU compatibility route; native playback uses these static primary targets.
pub(crate) fn blend_shape_data(stage: &Stage, mesh_path: &Path) -> Option<BlendShapeData> {
    let api = SkelBindingAPI::get(stage, mesh_path.clone()).ok()??;
    let names = api.blend_shapes().ok()?;
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
                .ok()?
                .into_iter()
                .map(|v| [v.x, v.y, v.z])
                .collect(),
        );
        data.normal_offsets.push(
            shape
                .normal_offsets()
                .ok()?
                .into_iter()
                .map(|v| [v.x, v.y, v.z])
                .collect(),
        );
        data.point_indices.push(shape.point_indices().ok()?);
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
        SkelAnimQuery::new(stage, anim_path),
        Ok(Some(anim)) if anim.joint_transforms_might_be_time_varying()
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
    let joints = skeleton.joints()?;
    let skinning = SkinningResolver::from_binding(&binding.binding, &joints)?;
    if skinning.is_rigidly_deformed() || !skinning.has_joint_influences() {
        return Ok(None);
    }
    let resolver = SkeletonResolver::from_skeleton(&skeleton)?;

    // Joint-local transforms at `time`: from the bound SkelAnimation, else the
    // skeleton's rest pose (which yields the undeformed mesh).
    let rest = || resolver.rest_pose_local().to_vec();
    let anim_path = animation_source(stage, &skel_path, &binding);
    let locals: Vec<gf::Matrix4d> = match anim_path {
        Some(anim_path) => match SkelAnimQuery::new(stage, anim_path)? {
            Some(anim) => {
                let tc = TimeCode::new(time.unwrap_or(0.0));
                let l = anim.compute_joint_local_transforms(stage, tc)?;
                // Order must line up with the skeleton; otherwise fall back to
                // rest rather than deform incorrectly.
                if l.len() == joints.len() { l } else { rest() }
            }
            None => rest(),
        },
        None => rest(),
    };

    let skel_xforms =
        resolver.compute_skinning_transforms_from_local(&locals, gf::Matrix4d::IDENTITY);

    let Some(mesh) = super::geom::read_mesh(stage, mesh_path)? else {
        return Ok(None);
    };
    // Canonical UsdSkel order: morph blend shapes first, then skin the result.
    let rest = blend_shape_deform(stage, mesh_path, &mesh.points, time).unwrap_or(mesh.points);
    let pts: Vec<gf::Vec3f> = rest.iter().map(|p| gf::Vec3f::from(*p)).collect();
    let d = skinning.compute_skinned_points(&pts, &skel_xforms);
    Ok(Some(d.into_iter().map(|v| [v.x, v.y, v.z]).collect()))
}
