use openusd::sdf::Path;
use usd_schema::StageReadExt;

pub fn probe_skinned_meshes(stage: &openusd::usd::Stage) {
    println!();
    println!("== skinned meshes ==");
    let mut count_skinned = 0;
    let mut count_with_subsets = 0;
    fn walk_skin(
        stage: &openusd::usd::Stage,
        prim: &Path,
        count_skinned: &mut usize,
        count_with_subsets: &mut usize,
    ) {
        let tn: String = stage
            .composed_field::<String>(prim.clone(), "typeName")
            .ok()
            .flatten()
            .unwrap_or_default();
        if tn == "Mesh" {
            if let Ok(Some(b)) = usd_schema::skel::read_skel_binding(stage, prim) {
                *count_skinned += 1;
                let mesh_data = usd_schema::geom::read_mesh(stage, prim).ok().flatten();
                let subset_count = mesh_data.as_ref().map(|m| m.subsets.len()).unwrap_or(0);
                let pt_count = mesh_data.as_ref().map(|m| m.points.len()).unwrap_or(0);
                if subset_count > 0 {
                    *count_with_subsets += 1;
                }
                let max_idx = b.joint_indices.iter().max().copied().unwrap_or(0);
                let min_idx = b.joint_indices.iter().min().copied().unwrap_or(0);
                let weight_sum_per_vert: f32 = if b.elements_per_vertex > 0 {
                    let n = b.elements_per_vertex as usize;
                    let chunks = b.joint_weights.chunks(n);
                    let count = chunks.len();
                    if count > 0 {
                        b.joint_weights
                            .chunks(n)
                            .map(|c| c.iter().sum::<f32>())
                            .sum::<f32>()
                            / count as f32
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };
                println!(
                    "  {} subsets={} pts={} subset_joints={} per_vert={} idx_range={}..{} avg_wsum={:.3} skel={:?}",
                    prim.as_str(),
                    subset_count,
                    pt_count,
                    b.joint_subset.len(),
                    b.elements_per_vertex,
                    min_idx,
                    max_idx,
                    weight_sum_per_vert,
                    b.skeleton,
                );
            }
        }
        for child in stage.prim_children(prim.clone()).unwrap_or_default() {
            if let Ok(c) = prim.append_path(child.as_str()) {
                walk_skin(stage, &c, count_skinned, count_with_subsets);
            }
        }
    }
    for n in stage.root_prims().unwrap_or_default() {
        if let Ok(p) = Path::abs_root().append_path(n.as_str()) {
            walk_skin(stage, &p, &mut count_skinned, &mut count_with_subsets);
        }
    }
    println!("total skinned: {count_skinned}, of which with GeomSubset: {count_with_subsets}");
}

pub fn probe_bindings(stage: &openusd::usd::Stage) {
    println!();
    println!("== hair mesh binding probe ==");
    for mp in [
        "/Skel/Geometry/HumanFemaleHair/Geom/Hair/Layers/HeadHair/BetaLeft_HairLayer/Standin/Shell_sbdv",
        "/Skel/Geometry/HumanFemaleHair/Geom/Hair/Layers/EyeHair/BrowL_HairLayer/Standin/Shell_sbdv",
    ] {
        let prim = match openusd::sdf::Path::new(mp) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if let Ok(Some(b)) = usd_schema::skel::read_skel_binding(stage, &prim) {
            let max_idx = b.joint_indices.iter().max().copied().unwrap_or(0);
            let min_idx = b.joint_indices.iter().min().copied().unwrap_or(0);
            let mut distinct: std::collections::BTreeSet<i32> =
                b.joint_indices.iter().copied().collect();
            let _ = distinct.split_off(&i32::MAX);
            let m = usd_schema::geom::read_mesh(stage, &prim).ok().flatten();
            let mut center = [0.0; 3];
            if let Some(ref m) = m {
                let mut mn = [f32::INFINITY; 3];
                let mut mx = [f32::NEG_INFINITY; 3];
                for p in &m.points {
                    for i in 0..3 {
                        if p[i] < mn[i] {
                            mn[i] = p[i];
                        }
                        if p[i] > mx[i] {
                            mx[i] = p[i];
                        }
                    }
                }
                for i in 0..3 {
                    center[i] = (mn[i] + mx[i]) * 0.5;
                }
            }
            println!(
                "  {mp}\n    points center=({:.1}, {:.1}, {:.1})\n    per_vert={} idx_range={}..{} distinct={:?}",
                center[0],
                center[1],
                center[2],
                b.elements_per_vertex,
                min_idx,
                max_idx,
                distinct.iter().take(8).copied().collect::<Vec<_>>(),
            );
        }
    }

    println!();
    println!("== nail/shoe binding probe ==");
    for mp in [
        "/Skel/Geometry/HumanFemale/Geom/Body/Nails/LFingerNails/ThumbNail_sbdv",
        "/Skel/Geometry/ShoesHumanFlats/Geom/LShoe/Body/ShoeBody_sbdv",
        "/Skel/Geometry/ShoesHumanFlats/Geom/LShoe/Sole/Sole_sbdv",
        "/Skel/Geometry/HumanFemaleHair/Geom/Hair/Hair_sbdv",
    ] {
        let prim = match openusd::sdf::Path::new(mp) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if let Ok(Some(b)) = usd_schema::skel::read_skel_binding(stage, &prim) {
            let mut idx_set: std::collections::BTreeSet<i32> =
                b.joint_indices.iter().copied().collect();
            let summary: Vec<i32> = idx_set.iter().take(8).copied().collect();
            let _ = idx_set.split_off(&i32::MAX);
            println!(
                "  {mp} per_vert={} joints={} skel:joints_authored={}",
                b.elements_per_vertex,
                summary.len(),
                b.joint_subset.len()
            );
            println!("    indices used: {:?}", summary);
        } else {
            println!("  {mp} → no binding");
        }
    }

    println!();
    println!("== hair inheritance walk ==");
    let mut cur =
        openusd::sdf::Path::new("/Skel/Geometry/HumanFemaleHair/Geom/Hair/Hair_sbdv").unwrap();
    loop {
        let attr = cur.append_property("primvars:skel:jointIndices").unwrap();
        let v = stage
            .composed_field::<openusd::sdf::Value>(attr, "default")
            .ok()
            .flatten();
        let count = match v {
            Some(openusd::sdf::Value::IntVec(ref v)) => Some(v.len()),
            _ => None,
        };
        let attr2 = cur.append_property("xformOp:transform").unwrap();
        let xf = stage
            .composed_field::<openusd::sdf::Value>(attr2, "default")
            .ok()
            .flatten();
        println!(
            "  {} → jointIndices {:?} xform {}",
            cur.as_str(),
            count,
            if xf.is_some() { "yes" } else { "no" }
        );
        match cur.parent() {
            Some(p) => cur = p,
            None => break,
        }
    }
}
