use bevy::prelude::Transform;
use project_protocol::ProjectStageTarget;
use usd_bevy::LiveStage;

use crate::project::cache_hydration::ActiveProjectCacheContext;
use crate::project::service::ActiveProjectStage;
use crate::viewport::api::CurrentHierarchyProjection;
use crate::viewport::semantic::SemanticSyncState;
use crate::viewport::session::StageInfo;

use super::ProductionActivationWorld;

impl ProductionActivationWorld {
    pub(crate) fn assert_empty_activation(
        &self,
        project_id: usd_project::ProjectId,
        target: ProjectStageTarget,
    ) {
        let world = self.world();
        assert!(world.get_non_send::<LiveStage>().is_none());
        assert!(world.get_resource::<ActiveProjectCacheContext>().is_none());
        let stage_info = world.resource::<StageInfo>();
        assert_eq!(stage_info.activation_generation, 4);
        assert!(stage_info.path.is_empty());
        let semantic = world.resource::<SemanticSyncState>();
        assert!(semantic.snapshot().is_none());
        assert!(semantic.shared_bim_index().is_none());
        assert!(
            world
                .resource::<CurrentHierarchyProjection>()
                .snapshot()
                .nodes
                .is_empty()
        );
        assert_eq!(
            self.active(),
            Some(ActiveProjectStage {
                project_id,
                target,
                generation: 4,
            })
        );
    }
}

pub(crate) fn seek_animation_signature(
    production: &mut ProductionActivationWorld,
    time_code: f64,
) -> u64 {
    {
        let mut clock = production
            .world_mut()
            .resource_mut::<crate::viewport::animation::UsdStageTime>();
        clock.playing = false;
        clock.seconds = (time_code - clock.start_time_code) / clock.time_codes_per_second;
    }
    production.update();
    production.update();
    let world = production.world_mut();
    let paths = world.resource::<usd_bevy::PathStore>();
    let map = world.resource::<usd_bevy::PrimEntities>();
    let mut samples = world
        .resource::<usd_bevy::AnimatedPrims>()
        .0
        .iter()
        .filter_map(|path| {
            let entity = map.entity(paths, path)?;
            Some((path.clone(), *world.get::<Transform>(entity)?))
        })
        .collect::<Vec<_>>();
    let mut joints = world.query::<(&usd_bevy::route::skel::UsdJoint, &Transform)>();
    samples.extend(
        joints
            .iter(world)
            .map(|(joint, transform)| (format!("joint:{}", joint.path), *transform)),
    );
    samples.sort_by(|(left, _), (right, _)| left.cmp(right));
    transform_signature(&samples)
}

impl ProductionActivationWorld {
    pub(crate) fn seek_animation_signature(&mut self, time_code: f64) -> u64 {
        seek_animation_signature(self, time_code)
    }
}

fn transform_signature(samples: &[(String, Transform)]) -> u64 {
    const FNV_OFFSET: u64 = 14_695_981_039_346_656_037;
    const FNV_PRIME: u64 = 1_099_511_628_211;
    let mut hash = FNV_OFFSET;
    for (path, transform) in samples {
        for byte in (path.len() as u64)
            .to_le_bytes()
            .iter()
            .chain(path.as_bytes())
        {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME);
        }
        for value in transform
            .translation
            .to_array()
            .into_iter()
            .chain(transform.rotation.to_array())
            .chain(transform.scale.to_array())
        {
            let quantized = (f64::from(value) * 1_000_000.0).round() as i64;
            for byte in quantized.to_le_bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME);
            }
        }
    }
    hash
}
