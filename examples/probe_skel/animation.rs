use openusd::sdf::Path;
use usd_schema::StageReadExt;

pub fn probe_animations(stage: &openusd::usd::Stage) {
    println!();
    println!("== anim vs bind translation scale check ==");
    let walk_path = "assets/UsdSkelExamples/HumanFemale/HumanFemale.walk.usd";
    if let Ok(walk_text) = std::fs::read_to_string(walk_path) {
        let anims = usd_schema::skel_anim_text::scan_skel_animations(&walk_text);
        if let Some(anim) = anims.first() {
            let anim_hips = anim.joints.iter().position(|j| j == "Hips");
            let mut skel_hips_rest = None;
            fn find_skel(
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
                        if let Some(s) = find_skel(stage, &cp) {
                            return Some(s);
                        }
                    }
                }
                None
            }
            for n in stage.root_prims().unwrap_or_default() {
                if let Ok(p) = openusd::sdf::Path::abs_root().append_path(n.as_str()) {
                    if let Some(s) = find_skel(stage, &p) {
                        skel_hips_rest = Some(s);
                        break;
                    }
                }
            }
            if let (Some(ai), Some(skel)) = (anim_hips, skel_hips_rest) {
                let skel_hips_local = bevy::math::Mat4::from_cols_array(&skel.rest_transforms[0]);
                let (_, _, sk_t) = skel_hips_local.to_scale_rotation_translation();
                let anim_first = anim.translations.iter().next();
                if let Some((_, vals)) = anim_first {
                    let an_t = vals[ai];
                    println!(
                        "  Hips skel rest_local_t = {sk_t:?} | anim translation_at_first = {an_t:?}"
                    );
                }
                if let Some((_, rot_vals)) = anim.rotations.iter().next() {
                    let ar = rot_vals[ai];
                    let aq = bevy::math::Quat::from_xyzw(ar[1], ar[2], ar[3], ar[0]);
                    let (_, sk_r, _) = skel_hips_local.to_scale_rotation_translation();
                    println!(
                        "  Hips skel rest_local_rot = {sk_r:?} | anim rotation_at_first (wxyz) = {ar:?} → quat {aq:?}"
                    );
                    let dot = sk_r.dot(aq).abs();
                    println!(
                        "  Hips dot(rest_rot, anim_rot) = {dot:.4} (1.0 = same; far from 1 = unit/order mismatch)"
                    );
                }
                if let Some((_, sc_vals)) = anim.scales.iter().next() {
                    let ascale = sc_vals[ai];
                    println!("  Hips anim scale_at_first = {ascale:?}");
                }
            }
        }
    }

    println!();
    println!("== anim-order joint lookup ==");
    if let Ok(walk_text) =
        std::fs::read_to_string("assets/UsdSkelExamples/HumanFemale/HumanFemale.walk.usd")
    {
        let anims = usd_schema::skel_anim_text::scan_skel_animations(&walk_text);
        if let Some(a) = anims.first() {
            println!("anim joint count: {}", a.joints.len());
            for ix in [
                55, 56, 57, 58, 59, 60, 100, 101, 102, 103, 104, 105, 106, 107, 108,
            ] {
                if ix < a.joints.len() {
                    println!("  [{ix}] {}", a.joints[ix]);
                }
            }
        }
    }
}
