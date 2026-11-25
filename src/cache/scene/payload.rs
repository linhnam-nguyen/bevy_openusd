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
            ensure!(
                sample.time.is_finite(),
                "Scene animation time is not finite"
            );
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
pub(crate) fn prepare_animation_payload(stage: &Stage, mesh_path: &str) -> Result<Option<Vec<u8>>> {
    use openusd::usd::SchemaBase;
    let mesh_path = sdf::path(mesh_path)?;
    let Some(query) = usd_bevy::read::skel::animation_query_for_mesh(stage, &mesh_path) else {
        return Ok(None);
    };
    let source_path = query.animation().prim().path().as_str().to_owned();
    let animation_path = sdf::path(source_path.as_str())
        .context("Scene animation query returned an invalid source path")?;
    let mut times = Vec::new();
    for property in ["translations", "rotations", "scales", "blendShapeWeights"] {
        let attribute = stage.attribute(animation_path.append_property(property)?)?;
        times.extend(attribute.time_sample_times()?);
    }
    times.retain(|time| time.is_finite());
    times.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    times.dedup_by(|left, right| left == right);
    if times.is_empty() {
        times.push(0.0);
    }

    let translations_authored = stage
        .attribute(animation_path.append_property("translations")?)?
        .has_authored_value()?;
    let rotations_authored = stage
        .attribute(animation_path.append_property("rotations")?)?
        .has_authored_value()?;
    let scales_authored = stage
        .attribute(animation_path.append_property("scales")?)?
        .has_authored_value()?;
    let joint_count = query.joint_order().len();

    let samples = times
        .iter()
        .map(|time| {
            let tc = TimeCode::new(*time);
            let (translations, rotations, scales) =
                if let Some((translations, rotations, scales)) = query.joint_transform_components(tc)?
                {
                    (
                        translations.into_iter().map(Into::into).collect(),
                        rotations.into_iter().map(Into::into).collect(),
                        scales.into_iter().map(Into::into).collect(),
                    )
                } else if !translations_authored && !rotations_authored && !scales_authored {
                    let identity_rotation: [f32; 4] = openusd::gf::Quatf::IDENTITY.into();
                    (
                        vec![[0.0, 0.0, 0.0]; joint_count],
                        vec![identity_rotation; joint_count],
                        vec![[1.0, 1.0, 1.0]; joint_count],
                    )
                } else {
                    anyhow::bail!("animation sample missing joint transform components");
                };
            let blend_shape_weights = query
                .blend_shape_weights(tc)?
                .unwrap_or_default();
            Ok(SceneAnimationSample {
                time: *time,
                translations,
                rotations,
                scales,
                blend_shape_weights,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let payload = SceneAnimationBlob {
        version: SCENE_ANIMATION_VERSION,
        source_path,
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
#[path = "../../../tests/unit/project/cache/scene/payload_tests.rs"]
mod tests;
