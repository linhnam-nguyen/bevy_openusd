//! Deferred metadata inspection over the already projected prim index.

use openusd::usd::Stage;
use usd_bevy::{PathStore, PrimEntities};

use crate::viewport::session::{
    StageCameraData, StageCameraInfo, StageCameraProjection, StageInfo, VariantSetInfo,
};

/// Refresh cameras and variant selections without forcing composition of an
/// unprojected stage. Projection remains the only operation that discovers
/// the composed prim namespace.
pub(super) fn refresh(
    stage: &Stage,
    prims: &PrimEntities,
    paths: &PathStore,
    info: &mut StageInfo,
) {
    let mut variants = std::collections::HashMap::new();
    let mut cameras = Vec::new();
    for (path, _) in prims.iter(paths) {
        if path == "/" {
            continue;
        }
        let Ok(prim_path) = openusd::sdf::path(path) else {
            continue;
        };
        let prim = stage.prim(prim_path);
        if let Ok(Some(type_name)) = prim.type_name()
            && type_name.as_str() == "Camera"
        {
            cameras.push(StageCameraInfo {
                path: path.to_owned(),
                data: StageCameraData {
                    focal_length_mm: Some(50.0),
                    projection: Some(StageCameraProjection::Perspective),
                },
            });
        }
        if let Ok(selections) = prim.variant_sets().get_all_variant_selections()
            && !selections.is_empty()
        {
            variants.insert(
                path.to_owned(),
                selections
                    .into_iter()
                    .map(|(name, selection)| VariantSetInfo {
                        name,
                        selection: Some(selection),
                        options: Vec::new(),
                    })
                    .collect(),
            );
        }
    }
    cameras.sort_by(|left, right| left.path.cmp(&right.path));
    let variant_count = variants.values().map(Vec::len).sum();
    info.variant_count = variant_count;
    info.variants = variants;
    info.cameras = cameras;
}
