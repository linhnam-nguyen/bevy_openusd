//! Versioned renderer-neutral Scene animation payloads.

use anyhow::{Context, Result, ensure};
use openusd::sdf;
use openusd::usd::{Stage, TimeCode};
use serde::{Deserialize, Serialize};

pub(crate) const SCENE_ANIMATION_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct SceneAnimationBlob {
    pub(crate) version: u16,
    pub(crate) source_path: String,
    pub(crate) joint_order: Vec<String>,
    pub(crate) blend_shape_order: Vec<String>,
    pub(crate) samples: Vec<SceneAnimationSample>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct SceneAnimationSample {
    pub(crate) time: f64,
    /// Quaternion components use OpenUSD's `(w, x, y, z)` order.
    pub(crate) translations: Vec<[f32; 3]>,
    pub(crate) rotations: Vec<[f32; 4]>,
    pub(crate) scales: Vec<[f32; 3]>,
    pub(crate) blend_shape_weights: Vec<f32>,
}

impl SceneAnimationBlob {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.version == SCENE_ANIMATION_VERSION,
            "unsupported Scene animation blob version {}",
            self.version
        );
        ensure!(!self.samples.is_empty(), "Scene animation has no samples");
        let joint_count = self.joint_order.len();
        let blend_shape_count = self.blend_shape_order.len();
        for sample in &self.samples {
            ensure!(sample.time.is_finite(), "Scene animation time is not finite");
            ensure!(
                sample.translations.len() == joint_count
                    && sample.rotations.len() == joint_count
                    && sample.scales.len() == joint_count,
                "Scene animation joint sample length does not match joint order"
            );
            ensure!(
                sample.blend_shape_weights.len() == blend_shape_count,
                "Scene animation blend-shape sample length does not match blend-shape order"
            );
            ensure!(
                sample
                    .translations
                    .iter()
                    .flatten()
                    .chain(sample.rotations.iter().flatten())
                    .chain(sample.scales.iter().flatten())
                    .chain(sample.blend_shape_weights.iter())
                    .all(|value| value.is_finite()),
                "Scene animation contains a non-finite value"
            );
        }
        Ok(())
    }
}

/// Extract authored animation sample times for one owned mesh and encode the
/// evaluated typed components. The source stage remains authoritative; this
/// is a deterministic cache/residency representation.
pub(crate) fn prepare_animation_payload(
    stage: &Stage,
    mesh_path: &str,
) -> Result<Option<Vec<u8>>> {
    let mesh_path = sdf::path(mesh_path)?;
    let Some(query) = usd_bevy::read::skel::animation_query_for_mesh(stage, &mesh_path) else {
        return Ok(None);
    };
    let animation_path = sdf::path(query.prim_path())
        .context("Scene animation query returned an invalid source path")?;
    let mut times = Vec::new();
    for property in ["translations", "rotations", "scales", "blendShapeWeights"] {
        let attribute = stage.attribute(animation_path.append_property(property)?);
        times.extend(attribute.time_sample_times()?);
    }
    times.retain(|time| time.is_finite());
    times.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    times.dedup_by(|left, right| left == right);
    if times.is_empty() {
        times.push(0.0);
    }

    let samples = times
        .iter()
        .map(|time| {
            let (translations, rotations, scales) = query
                .compute_joint_local_transform_components(stage, TimeCode::new(*time))?;
            let blend_shape_weights = query
                .compute_blend_shape_weights(stage, TimeCode::new(*time))?;
            Ok(SceneAnimationSample {
                time: *time,
                translations: translations.into_iter().map(Into::into).collect(),
                rotations: rotations.into_iter().map(Into::into).collect(),
                scales: scales.into_iter().map(Into::into).collect(),
                blend_shape_weights,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let payload = SceneAnimationBlob {
        version: SCENE_ANIMATION_VERSION,
        source_path: query.prim_path().to_owned(),
        joint_order: query.joint_order().to_vec(),
        blend_shape_order: query.blend_shape_order().to_vec(),
        samples,
    };
    payload.validate()?;
    Ok(Some(
        serde_json::to_vec(&payload).context("encode Scene animation payload")?,
    ))
}

#[cfg(test)]
mod tests {
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
}
