use openusd::sdf::Path;
use openusd::sdf::Value;
use usd_schema::StageReadExt;

pub fn probe_ancestors_and_blendshapes(stage: &openusd::usd::Stage) {
    println!();
    println!("== shoe ancestor xforms ==");
    let mut cur =
        openusd::sdf::Path::new("/Skel/Geometry/ShoesHumanFlats/Geom/LShoe/Body/ShoeBody_sbdv")
            .unwrap();
    loop {
        let order_attr = cur.append_property("xformOpOrder").unwrap();
        let order = stage
            .composed_field::<openusd::sdf::Value>(order_attr, "default")
            .ok()
            .flatten();
        let scale_attr = cur.append_property("xformOp:scale").unwrap();
        let scale = stage
            .composed_field::<openusd::sdf::Value>(scale_attr, "default")
            .ok()
            .flatten();
        let xform_attr = cur.append_property("xformOp:transform").unwrap();
        let xform = stage
            .composed_field::<openusd::sdf::Value>(xform_attr, "default")
            .ok()
            .flatten();
        println!(
            "  {} order={:?} scale={:?} transform={}",
            cur.as_str(),
            order.is_some(),
            scale,
            if xform.is_some() { "yes" } else { "no" }
        );
        match cur.parent() {
            Some(p) => cur = p,
            None => break,
        }
    }

    println!();
    println!("== ShoesHumanFlats xform ==");
    if let Ok(Some(t)) = usd_schema::xform::read_transform(
        stage,
        &openusd::sdf::Path::new("/Skel/Geometry/ShoesHumanFlats").unwrap(),
    ) {
        println!("  translate {:?}", t.translate);
        println!("  rotate {:?}", t.rotate);
        println!("  scale {:?}", t.scale);
    }

    println!();
    println!("== bound joint world bind translations ==");
    fn find_first_skel(
        stage: &openusd::usd::Stage,
        p: &openusd::sdf::Path,
    ) -> Option<usd_schema::skel::ReadSkeleton> {
        let tn = stage
            .composed_field::<String>(p.clone(), "typeName")
            .ok()
            .flatten()
            .unwrap_or_default();
        if tn == "Skeleton" {
            return usd_schema::skel::read_skeleton(stage, p).ok().flatten();
        }
        for c in stage.prim_children(p.clone()).unwrap_or_default() {
            if let Ok(cp) = p.append_path(c.as_str()) {
                if let Some(s) = find_first_skel(stage, &cp) {
                    return Some(s);
                }
            }
        }
        None
    }
    let mut skel_for_probe = None;
    for n in stage.root_prims().unwrap_or_default() {
        if let Ok(p) = openusd::sdf::Path::abs_root().append_path(n.as_str()) {
            if let Some(s) = find_first_skel(stage, &p) {
                skel_for_probe = Some(s);
                break;
            }
        }
    }
    if let Some(skel) = skel_for_probe {
        for ix in [0_usize, 100, 101, 102, 103] {
            if let Some(m) = skel.bind_transforms.get(ix) {
                let mat = bevy::math::Mat4::from_cols_array(m);
                let (_, _, t) = mat.to_scale_rotation_translation();
                let name = skel.joints.get(ix).cloned().unwrap_or_default();
                println!("  [{ix}] {name} bind_t = {t:?}");
            }
        }
    }

    println!();
    println!("== blendshape census ==");
    let mut bs_meshes = 0usize;
    let mut bs_total = 0usize;
    let mut bs_max_per_mesh = 0usize;
    let mut bs_sparse = 0usize;
    let mut bs_dense = 0usize;
    let mut bs_max_offsets = 0usize;
    let mut printed_examples = 0;
    fn probe_bs(
        stage: &openusd::usd::Stage,
        prim: &Path,
        bs_meshes: &mut usize,
        bs_total: &mut usize,
        bs_max_per_mesh: &mut usize,
        bs_sparse: &mut usize,
        bs_dense: &mut usize,
        bs_max_offsets: &mut usize,
        printed_examples: &mut usize,
    ) {
        let tn = stage
            .composed_field::<String>(prim.clone(), "typeName")
            .ok()
            .flatten()
            .unwrap_or_default();
        if tn == "Mesh" {
            if let Ok(Some(b)) = usd_schema::skel::read_skel_binding(stage, prim) {
                if !b.blend_shape_targets.is_empty() {
                    *bs_meshes += 1;
                    *bs_total += b.blend_shape_targets.len();
                    *bs_max_per_mesh = (*bs_max_per_mesh).max(b.blend_shape_targets.len());
                    if *printed_examples < 2 {
                        println!(
                            "  {} → {} blend_shape_targets, {} blend_shapes (names)",
                            prim.as_str(),
                            b.blend_shape_targets.len(),
                            b.blend_shapes.len(),
                        );
                        *printed_examples += 1;
                    }
                    for t in b.blend_shape_targets.iter().take(3) {
                        let bs_path = match openusd::sdf::Path::new(t) {
                            Ok(p) => p,
                            Err(_) => continue,
                        };
                        if let Ok(Some(bs)) = usd_schema::skel::read_blend_shape(stage, &bs_path) {
                            if bs.point_indices.is_empty() {
                                *bs_dense += 1;
                            } else {
                                *bs_sparse += 1;
                            }
                            *bs_max_offsets = (*bs_max_offsets).max(bs.offsets.len());
                        }
                    }
                }
            }
        }
        for c in stage.prim_children(prim.clone()).unwrap_or_default() {
            if let Ok(cp) = prim.append_path(c.as_str()) {
                probe_bs(
                    stage,
                    &cp,
                    bs_meshes,
                    bs_total,
                    bs_max_per_mesh,
                    bs_sparse,
                    bs_dense,
                    bs_max_offsets,
                    printed_examples,
                );
            }
        }
    }
    for n in stage.root_prims().unwrap_or_default() {
        if let Ok(p) = openusd::sdf::Path::abs_root().append_path(n.as_str()) {
            probe_bs(
                stage,
                &p,
                &mut bs_meshes,
                &mut bs_total,
                &mut bs_max_per_mesh,
                &mut bs_sparse,
                &mut bs_dense,
                &mut bs_max_offsets,
                &mut printed_examples,
            );
        }
    }
    println!(
        "blendshape summary: {bs_meshes} meshes have blendshapes, total target refs={bs_total}, max-per-mesh={bs_max_per_mesh}"
    );
    println!("sampled targets: dense={bs_dense} sparse={bs_sparse} max_offsets={bs_max_offsets}");

    println!();
    println!("== mesh point space probe ==");
    for mp in [
        "/Skel/Geometry/HumanFemale/Geom/Body/Body_sbdv",
        "/Skel/Geometry/HumanFemale/Geom/Body/Nails/LFingerNails/ThumbNail_sbdv",
        "/Skel/Geometry/ShoesHumanFlats/Geom/LShoe/Body/ShoeBody_sbdv",
    ] {
        let prim = match openusd::sdf::Path::new(mp) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if let Ok(Some(m)) = usd_schema::geom::read_mesh(stage, &prim) {
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
            let cx = (mn[0] + mx[0]) * 0.5;
            let cy = (mn[1] + mx[1]) * 0.5;
            let cz = (mn[2] + mx[2]) * 0.5;
            println!(
                "  {mp} center=({cx:.2}, {cy:.2}, {cz:.2}) extent=({:.2}, {:.2}, {:.2})",
                mx[0] - mn[0],
                mx[1] - mn[1],
                mx[2] - mn[2],
            );
        }
    }

    println!();
    println!("== full skinned-mesh census ==");
    fn census(
        stage: &openusd::usd::Stage,
        prim: &Path,
        out: &mut Vec<(Path, Option<usd_schema::skel::ReadSkelBinding>, usize)>,
    ) {
        let tn = stage
            .composed_field::<String>(prim.clone(), "typeName")
            .ok()
            .flatten()
            .unwrap_or_default();
        if tn == "Mesh" {
            let binding = usd_schema::skel::read_skel_binding(stage, prim)
                .ok()
                .flatten();
            let pts = usd_schema::geom::read_mesh(stage, prim)
                .ok()
                .flatten()
                .map(|m| m.points.len())
                .unwrap_or(0);
            out.push((prim.clone(), binding, pts));
        }
        for c in stage.prim_children(prim.clone()).unwrap_or_default() {
            if let Ok(cp) = prim.append_path(c.as_str()) {
                census(stage, &cp, out);
            }
        }
    }
    let mut all = Vec::new();
    for n in stage.root_prims().unwrap_or_default() {
        if let Ok(p) = openusd::sdf::Path::abs_root().append_path(n.as_str()) {
            census(stage, &p, &mut all);
        }
    }
    let total = all.len();
    let with_binding = all.iter().filter(|(_, b, _)| b.is_some()).count();
    let mut by_purpose: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for (path, _, _) in &all {
        let purpose = usd_schema::geom::read_purpose(stage, path)
            .ok()
            .unwrap_or_else(|| "default".into());
        *by_purpose.entry(purpose).or_insert(0) += 1;
    }
    println!("mesh purpose distribution: {by_purpose:?}");
    let _ = std::fs::write(
        "/tmp/all_meshes.txt",
        all.iter()
            .map(|(p, _, _)| p.as_str().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    );
    println!(
        "total mesh prims: {total}, with SkelBindingAPI: {with_binding}, without: {}",
        total - with_binding
    );
    println!("first 8 unbound meshes:");
    for (path, b, pts) in all.iter().filter(|(_, b, _)| b.is_none()).take(8) {
        println!("  {} ({pts} pts) {b:?}", path.as_str());
    }

    let mut subsetted_skinned = 0usize;
    let mut subsetted_paths = Vec::new();
    for (path, _, _) in all.iter().filter(|(_, b, _)| b.is_some()) {
        if let Ok(Some(m)) = usd_schema::geom::read_mesh(stage, path) {
            if !m.subsets.is_empty() {
                subsetted_skinned += 1;
                if subsetted_paths.len() < 5 {
                    subsetted_paths.push(path.clone());
                }
            }
        }
    }
    println!("bound meshes with subsets (skin gets bypassed): {subsetted_skinned}");
    for p in &subsetted_paths {
        println!("  {}", p.as_str());
    }

    println!();
    println!("== inherited skel:joints walk ==");
    let mut cur = Path::new("/Skel/Geometry/HumanFemale/Geom/Body/Body_sbdv").unwrap();
    loop {
        let attr = cur.append_property("skel:joints").unwrap();
        let v = stage.composed_field::<Value>(attr, "default").ok().flatten();
        let count = match v {
            Some(Value::TokenVec(ref t)) => Some(t.len()),
            Some(Value::StringVec(ref t)) => Some(t.len()),
            _ => None,
        };
        println!("  {} → skel:joints {:?}", cur.as_str(), count);
        match cur.parent() {
            Some(p) => cur = p,
            None => break,
        }
    }

    println!();
    println!("== geomBindTransform probe ==");
    for mp in [
        "/Skel/Geometry/HumanFemale/Geom/Body/Body_sbdv",
        "/Skel/Geometry/HumanFemale/Geom/Body/Nails/LFingerNails/ThumbNail_sbdv",
    ] {
        let prim = Path::new(mp).unwrap();
        let attr = prim
            .append_property("primvars:skel:geomBindTransform")
            .unwrap();
        let v = stage
            .composed_field::<Value>(attr, "default")
            .ok()
            .flatten();
        println!(
            "  {mp} → primvars:skel:geomBindTransform = {:?}",
            v.is_some()
        );
        if let Some(val) = v {
            match val {
                Value::Matrix4d(m) => {
                    println!("    Matrix4d row0: {:?}", &m.0[0..4]);
                    println!("    Matrix4d row1: {:?}", &m.0[4..8]);
                    println!("    Matrix4d row2: {:?}", &m.0[8..12]);
                    println!("    Matrix4d row3: {:?}", &m.0[12..16]);
                }
                other => println!("    other variant: {other:?}"),
            }
        }
    }
}
