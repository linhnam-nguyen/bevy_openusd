//! Opt-in scene and physics diagnostics for the native viewport.

use bevy::prelude::*;
use usd_bevy::UsdPrimRef;

use crate::viewport::scene::visualization::SceneExtent;

/// One-shot dump of every prim entity's path, world translation, and
/// whether it carries a Mesh3d. Written to `/tmp/kitchen_layout.txt`
/// when `BEVY_OPENUSD_DEBUG_LAYOUT=1`. Used for offline analysis when
/// a real production asset (Pixar Kitchen_set, etc.) loads with
/// scattered geometry — clustering by parent path quickly shows
/// whether transforms are off, payloads failed, or a specific prop
/// landed at the wrong scale.
/// Emits a one-time diagnostic dump of spawned transforms and hierarchy.
pub(crate) fn debug_dump_layout_once(
    prims: Query<(
        &UsdPrimRef,
        &GlobalTransform,
        &Transform,
        Option<&bevy::mesh::Mesh3d>,
    )>,
    extent: Res<SceneExtent>,
    mut done: Local<bool>,
) {
    if *done || extent.count == 0 {
        return;
    }
    if std::env::var("BEVY_OPENUSD_DEBUG_LAYOUT").ok().as_deref() != Some("1") {
        *done = true;
        return;
    }
    let mut rows: Vec<(String, Vec3, Vec3, Quat, Vec3, bool)> = prims
        .iter()
        .map(|(p, gt, t, mesh)| {
            (
                p.path.clone(),
                gt.translation(),
                t.translation,
                t.rotation,
                t.scale,
                mesh.is_some(),
            )
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = String::new();
    out.push_str(&format!(
        "# {} prims · {} with Mesh3d · diag {:.2} m\n# path | world_xyz | local_t | local_quat(xyzw) | local_scale | has_mesh\n",
        rows.len(),
        rows.iter().filter(|r| r.5).count(),
        extent.diag()
    ));
    for (path, w, lt, lq, ls, has_mesh) in &rows {
        out.push_str(&format!(
            "{path} | {:.3},{:.3},{:.3} | {:.3},{:.3},{:.3} | {:.3},{:.3},{:.3},{:.3} | {:.3},{:.3},{:.3} | {}\n",
            w.x, w.y, w.z,
            lt.x, lt.y, lt.z,
            lq.x, lq.y, lq.z, lq.w,
            ls.x, ls.y, ls.z,
            has_mesh
        ));
    }
    let path = "/tmp/kitchen_layout.txt";
    if let Err(e) = std::fs::write(path, &out) {
        info!("layout dump failed: {e}");
    } else {
        info!("layout dump: wrote {} rows to {path}", rows.len());
    }
    *done = true;
}

/// One-shot dump of every rigid body's world pose + every joint's
/// authored frames + computed world-anchor positions on each side.
/// Helps verify the projection got the joint topology right BEFORE
/// physics has any chance to touch the bodies. Also reports, for
/// every visual mesh, whether a `UsdRigidBody` ancestor exists in
/// the Bevy hierarchy — that's what lets Rapier's body movement
/// drag the mesh along after a step. Set
/// `BEVY_OPENUSD_DEBUG_PHYSICS=1` to enable.
/// Emits a one-time diagnostic summary of imported USD physics components.
pub(crate) fn debug_dump_physics_once(
    bodies: Query<(&UsdPrimRef, &GlobalTransform), With<usd_bevy::UsdRigidBody>>,
    joints: Query<(&UsdPrimRef, &usd_bevy::UsdPhysicsJoint)>,
    body_transforms: Query<&GlobalTransform>,
    meshes: Query<(Entity, Option<&UsdPrimRef>), With<bevy::mesh::Mesh3d>>,
    colliders: Query<(Entity, Option<&UsdPrimRef>), With<usd_bevy::UsdCollider>>,
    rigid_bodies: Query<(), With<usd_bevy::UsdRigidBody>>,
    parents: Query<&ChildOf>,
    prim_refs: Query<&UsdPrimRef>,
    local_transforms: Query<&Transform>,
    extent: Res<SceneExtent>,
    mut done: Local<bool>,
) {
    if *done || extent.count == 0 {
        return;
    }
    if std::env::var("BEVY_OPENUSD_DEBUG_PHYSICS").ok().as_deref() != Some("1") {
        *done = true;
        return;
    }
    info!("==== DEBUG PHYSICS DUMP ====");
    info!("---- Rigid bodies (pre-physics world pose) ----");
    let mut body_rows: Vec<_> = bodies.iter().collect();
    body_rows.sort_by(|a, b| a.0.path.cmp(&b.0.path));
    for (pr, gt) in &body_rows {
        let t = gt.compute_transform();
        info!(
            "  body {:<40} pos={:>+7.4?} rot={:>+7.4?}",
            pr.path, t.translation, t.rotation
        );
    }
    info!("---- Joints (authored local frames + computed world anchors) ----");
    let mut joint_rows: Vec<_> = joints.iter().collect();
    joint_rows.sort_by(|a, b| a.0.path.cmp(&b.0.path));
    for (pr, joint) in &joint_rows {
        let body0_pose = joint.body0.and_then(|e| body_transforms.get(e).ok());
        let body1_pose = joint.body1.and_then(|e| body_transforms.get(e).ok());
        let world_anchor0 = body0_pose.map(|gt| {
            let t = gt.compute_transform();
            t.translation + t.rotation * joint.local_pos0
        });
        let world_anchor1 = body1_pose.map(|gt| {
            let t = gt.compute_transform();
            t.translation + t.rotation * joint.local_pos1
        });
        let agree = match (world_anchor0, world_anchor1) {
            (Some(a), Some(b)) => format!("Δ={:.5}", (a - b).length()),
            _ => "(missing body)".to_string(),
        };
        info!(
            "  joint {:<35} kind={:?} axis={:?}",
            pr.path, joint.kind, joint.axis
        );
        info!(
            "    body0={:?} local_pos0={:+.4?} local_rot0={:+.4?} → world_anchor0={:?}",
            joint.body0, joint.local_pos0, joint.local_rot0, world_anchor0
        );
        info!(
            "    body1={:?} local_pos1={:+.4?} local_rot1={:+.4?} → world_anchor1={:?}",
            joint.body1, joint.local_pos1, joint.local_rot1, world_anchor1
        );
        info!("    anchors {agree} (should be ~0)");
    }
    // Walk every visual mesh up its parent chain looking for a
    // `UsdRigidBody`. If a mesh has no body ancestor, Rapier moving
    // the body won't drag the mesh along — that's the "scattered
    // when physics enabled" symptom.
    let find_body_ancestor = |mut e: Entity| -> Option<(Entity, String)> {
        loop {
            if rigid_bodies.get(e).is_ok() {
                let p = prim_refs
                    .get(e)
                    .map(|pr| pr.path.clone())
                    .unwrap_or_else(|_| "<no path>".into());
                return Some((e, p));
            }
            match parents.get(e) {
                Ok(c) => e = c.parent(),
                Err(_) => return None,
            }
        }
    };
    info!("---- Visual meshes: full ancestor chain ----");
    let mut with = 0usize;
    let mut without = 0usize;
    let mut mesh_rows: Vec<_> = meshes.iter().collect();
    mesh_rows.sort_by_key(|(_, pr)| pr.map(|p| p.path.clone()).unwrap_or_default());
    for (e, pr) in &mesh_rows {
        let mesh_path = pr.map(|p| p.path.as_str()).unwrap_or("<no UsdPrimRef>");
        match find_body_ancestor(*e) {
            Some((_, body_path)) => {
                with += 1;
                info!("  mesh {mesh_path} (body ancestor {body_path}):");
                // Walk from this entity up to the body and print each
                // hop's local Transform — that's the chain Bevy
                // multiplies to produce the mesh's GlobalTransform
                // every frame. If any hop has a translation that
                // already encodes the body's world position (instead
                // of just an offset relative to its parent), then
                // moving the body will displace the visuals by that
                // amount.
                let mut cursor = *e;
                let mut depth = 0;
                loop {
                    let path = prim_refs
                        .get(cursor)
                        .map(|p| p.path.as_str().to_string())
                        .unwrap_or_else(|_| format!("<entity {:?}>", cursor));
                    let t = local_transforms.get(cursor).ok();
                    if let Some(t) = t {
                        info!(
                            "    [{depth}] {path}  local_t={:+.4?} local_r={:+.4?}",
                            t.translation, t.rotation
                        );
                    } else {
                        info!("    [{depth}] {path}  (no Transform)");
                    }
                    if rigid_bodies.get(cursor).is_ok() {
                        break;
                    }
                    match parents.get(cursor) {
                        Ok(c) => {
                            cursor = c.parent();
                            depth += 1;
                        }
                        Err(_) => break,
                    }
                }
            }
            None => {
                without += 1;
                info!("  mesh {mesh_path} → ⚠ NO body ancestor (won't follow physics)");
            }
        }
    }
    info!("  → {with} mesh(es) under a body, {without} mesh(es) NOT under any body");
    info!("---- Colliders vs. body ancestry ----");
    let mut col_rows: Vec<_> = colliders.iter().collect();
    col_rows.sort_by_key(|(_, pr)| pr.map(|p| p.path.clone()).unwrap_or_default());
    for (e, pr) in &col_rows {
        let col_path = pr.map(|p| p.path.as_str()).unwrap_or("<no UsdPrimRef>");
        match find_body_ancestor(*e) {
            Some((_, body_path)) => info!("  collider {col_path} → body ancestor {body_path}"),
            None => info!("  collider {col_path} → ⚠ NO body ancestor"),
        }
    }
    info!("==== END PHYSICS DUMP ====");
    *done = true;
}

/// Recurring sample of body+mesh world poses after physics has had a
/// chance to step. Lets us see whether visual meshes drift away from
/// their body ancestor over time. Set
/// `BEVY_OPENUSD_DEBUG_PHYSICS_TICK=1` to enable; emits every ~120
/// frames.
/// Periodically logs physics state while the optional runtime diagnostic is enabled.
pub(crate) fn debug_dump_physics_tick(
    bodies: Query<(&UsdPrimRef, &Transform, &GlobalTransform), With<usd_bevy::UsdRigidBody>>,
    meshes: Query<(Entity, &UsdPrimRef, &Transform, &GlobalTransform), With<bevy::mesh::Mesh3d>>,
    rigid_bodies: Query<(), With<usd_bevy::UsdRigidBody>>,
    parents: Query<&ChildOf>,
    prim_refs: Query<&UsdPrimRef>,
    mut counter: Local<u32>,
) {
    if std::env::var("BEVY_OPENUSD_DEBUG_PHYSICS_TICK")
        .ok()
        .as_deref()
        != Some("1")
    {
        return;
    }
    *counter += 1;
    if *counter % 120 != 0 {
        return;
    }
    let find_body_ancestor = |mut e: Entity| -> Option<(Entity, String)> {
        loop {
            if rigid_bodies.get(e).is_ok() {
                let p = prim_refs
                    .get(e)
                    .map(|pr| pr.path.clone())
                    .unwrap_or_else(|_| "<no path>".into());
                return Some((e, p));
            }
            match parents.get(e) {
                Ok(c) => e = c.parent(),
                Err(_) => return None,
            }
        }
    };
    info!("==== PHYSICS TICK SAMPLE (frame {}) ====", *counter);
    for (pr, t, gt) in &bodies {
        let gtt = gt.compute_transform();
        info!(
            "  body {} | local pos={:+.4?} rot={:+.4?} | global pos={:+.4?} rot={:+.4?}",
            pr.path, t.translation, t.rotation, gtt.translation, gtt.rotation
        );
    }
    for (e, pr, t, gt) in &meshes {
        let gtt = gt.compute_transform();
        let body = find_body_ancestor(e).map(|(_, p)| p).unwrap_or_default();
        info!(
            "  mesh {} (body {}) | local pos={:+.4?} rot={:+.4?} | global pos={:+.4?} rot={:+.4?}",
            pr.path, body, t.translation, t.rotation, gtt.translation, gtt.rotation
        );
    }
    info!("==== END TICK SAMPLE ====");
}

/// Dump geom-bearing prims that landed within ~1 % of the scene diagonal
/// from the world origin. Fires once on the first frame after the scene
/// materializes — a quick diagnostic for the "stuff stuck at origin" class
/// of bugs (missing xform ops, broken basis fix, etc.). Set
/// `BEVY_OPENUSD_DEBUG_ORIGIN=1` to enable.
/// Logs prims near the origin once to aid scene-placement debugging.
pub(crate) fn debug_origin_prims_once(
    prims: Query<(&UsdPrimRef, &GlobalTransform), With<bevy::mesh::Mesh3d>>,
    extent: Res<SceneExtent>,
    mut done: Local<bool>,
) {
    if *done || extent.count == 0 {
        return;
    }
    if std::env::var("BEVY_OPENUSD_DEBUG_ORIGIN").ok().as_deref() != Some("1") {
        *done = true;
        return;
    }
    let diag = extent.diag();
    let threshold = (diag * 0.01).max(0.05);
    let mut near_origin: Vec<(String, Vec3)> = Vec::new();
    for (prim, gt) in prims.iter() {
        let p = gt.translation();
        if p.length() < threshold {
            near_origin.push((prim.path.clone(), p));
        }
    }
    if near_origin.is_empty() {
        info!(
            "origin debug: no geom prims within {threshold:.3} m of (0,0,0) — \
             origin extrusion bug not reproduced"
        );
    } else {
        info!(
            "origin debug: {} geom prim(s) within {threshold:.3} m of (0,0,0):",
            near_origin.len()
        );
        for (path, pos) in near_origin.iter().take(40) {
            info!(
                "    {path}  @  ({:+.4}, {:+.4}, {:+.4})",
                pos.x, pos.y, pos.z
            );
        }
        if near_origin.len() > 40 {
            info!("    … and {} more", near_origin.len() - 40);
        }
    }
    *done = true;
}
