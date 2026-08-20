use bevy::math::Mat4;
use openusd::sdf::Path;
use usd_schema::StageReadExt;

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

pub fn probe_skeletons(stage: &openusd::usd::Stage) {
    fn walk(stage: &openusd::usd::Stage, prim: &Path) {
        let tn: String = stage
            .composed_field::<String>(prim.clone(), "typeName")
            .ok()
            .flatten()
            .unwrap_or_default();
        if tn == "Skeleton" {
            if let Ok(Some(s)) = usd_schema::skel::read_skeleton(stage, prim) {
                println!(
                    "Skeleton {} joints={} bind={} rest={}",
                    prim.as_str(),
                    s.joints.len(),
                    s.bind_transforms.len(),
                    s.rest_transforms.len(),
                );
                let parents = s.joint_parent_indices();
                let mut rest_world = vec![Mat4::IDENTITY; s.joints.len()];
                for i in 0..s.joints.len() {
                    let local = Mat4::from_cols_array(
                        &s.rest_transforms.get(i).copied().unwrap_or(IDENTITY),
                    );
                    rest_world[i] = match parents[i] {
                        Some(pi) => rest_world[pi] * local,
                        None => local,
                    };
                }
                let mut max_diff = 0.0f32;
                let mut worst = 0usize;
                for i in 0..s.joints.len() {
                    let bind_m = Mat4::from_cols_array(
                        &s.bind_transforms.get(i).copied().unwrap_or(IDENTITY),
                    );
                    let diff = (rest_world[i]
                        .to_cols_array_2d()
                        .iter()
                        .zip(bind_m.to_cols_array_2d().iter()))
                    .map(|(a, b)| {
                        a.iter()
                            .zip(b.iter())
                            .map(|(x, y)| (x - y).powi(2))
                            .sum::<f32>()
                    })
                    .sum::<f32>()
                    .sqrt();
                    if diff > max_diff {
                        max_diff = diff;
                        worst = i;
                    }
                }
                println!(
                    "  max rest_world vs bind matrix diff: {max_diff:.6} at joint [{worst}] {}",
                    s.joints[worst]
                );

                let mut decomp_max = 0.0f32;
                let mut decomp_worst = 0usize;
                for i in 0..s.joints.len() {
                    let bind_m = Mat4::from_cols_array(
                        &s.bind_transforms.get(i).copied().unwrap_or(IDENTITY),
                    );
                    let (s_, r_, t_) = bind_m.to_scale_rotation_translation();
                    let recomposed = Mat4::from_scale_rotation_translation(s_, r_, t_);
                    let diff = (bind_m
                        .to_cols_array()
                        .iter()
                        .zip(recomposed.to_cols_array().iter()))
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f32>()
                    .sqrt();
                    if diff > decomp_max {
                        decomp_max = diff;
                        decomp_worst = i;
                    }
                }
                println!(
                    "  decompose-recompose max drift: {decomp_max:.6} at joint [{decomp_worst}] {}",
                    s.joints[decomp_worst],
                );
                if decomp_max > 1e-3 {
                    let bind_m = Mat4::from_cols_array(
                        &s.bind_transforms
                            .get(decomp_worst)
                            .copied()
                            .unwrap_or(IDENTITY),
                    );
                    let det = bind_m.determinant();
                    println!("    determinant: {det:.6}");
                    let (s_, r_, t_) = bind_m.to_scale_rotation_translation();
                    println!("    decomposed: scale={s_:?} rot={r_:?} trans={t_:?}");
                }
                if max_diff > 1e-3 {
                    let bind_m = Mat4::from_cols_array(
                        &s.bind_transforms.get(worst).copied().unwrap_or(IDENTITY),
                    );
                    println!(
                        "  worst rest_world: {:?}",
                        rest_world[worst].to_cols_array()
                    );
                    println!("  worst bind:       {:?}", bind_m.to_cols_array());
                }
            }
        } else if tn == "SkelRoot" {
            if let Ok(Some(r)) = usd_schema::skel::read_skel_root(stage, prim) {
                println!(
                    "SkelRoot {} skel={:?} animSrc={:?}",
                    prim.as_str(),
                    r.skeleton,
                    r.animation_source,
                );
            }
        }
        for child in stage.prim_children(prim.clone()).unwrap_or_default() {
            if let Ok(child_path) = prim.append_path(child.as_str()) {
                walk(stage, &child_path);
            }
        }
    }

    for n in stage.root_prims().unwrap_or_default() {
        if let Ok(p) = Path::abs_root().append_path(n.as_str()) {
            walk(stage, &p);
        }
    }
}
