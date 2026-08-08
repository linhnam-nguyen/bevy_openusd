//! Skeleton visualization and temporary inspection modes.

use bevy::gizmos::config::{GizmoConfigGroup, GizmoConfigStore};
use bevy::prelude::*;
use bevy::reflect::Reflect;

use super::visualization::DisplayToggles;

/// Custom gizmo group for the skeleton overlay. Configured at
/// startup with `depth_bias = -1.0` so bone lines render in front of
/// the skin mesh — without that, a hummingbird rig is invisible
/// inside its own body.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct SkeletonGizmos;

/// Configures skeleton gizmos to render on top of skinned geometry.
pub(crate) fn setup_skeleton_gizmos_on_top(mut store: ResMut<GizmoConfigStore>) {
    let (cfg, _) = store.config_mut::<SkeletonGizmos>();
    cfg.depth_bias = -1.0;
}

/// Draw bone gizmos for every UsdSkel skeleton in the scene: a line
/// from each `UsdJoint` to each of its `UsdJoint` children. Drives
/// off `DisplayToggles.show_skeleton` (UI toggle + B hotkey) — and
/// also stays on whenever the legacy `BEVY_OPENUSD_JOINT_GIZMOS`
/// env var is set, in which case we additionally drop a green
/// sphere at every joint origin (the env-var path is the engine
/// debug view; the toggle path is what users actually want).
/// Draws the skeleton hierarchy when the skeleton overlay is enabled.
pub(crate) fn draw_joint_gizmos(
    mut gizmos: Gizmos<SkeletonGizmos>,
    joints: Query<(&GlobalTransform, Option<&Children>), With<usd_bevy::prim_ref::UsdJoint>>,
    children_q: Query<&GlobalTransform, With<usd_bevy::prim_ref::UsdJoint>>,
    flag: Res<ShowJointGizmosFlag>,
    toggles: Res<DisplayToggles>,
) {
    let want_bones = toggles.show_skeleton || flag.0;
    let want_spheres = flag.0;
    if !want_bones {
        return;
    }
    for (parent_gt, children) in joints.iter() {
        let parent_pos = parent_gt.translation();
        if want_spheres {
            gizmos.sphere(parent_pos, 0.01, bevy::color::palettes::tailwind::LIME_400);
        }
        if let Some(children) = children {
            for child in children.iter() {
                if let Ok(child_gt) = children_q.get(child) {
                    let child_pos = child_gt.translation();
                    gizmos.line(
                        parent_pos,
                        child_pos,
                        bevy::color::palettes::tailwind::CYAN_400,
                    );
                }
            }
        }
    }
}

/// Diagnostic: when `BEVY_OPENUSD_HIDE_MESHES=1` is set, hide every
/// mesh entity so the user only sees the skeleton via the gizmos
/// system. Lets us answer "is the rig animating?" without the
/// visual noise of broken skinning.
/// Hides meshes once when the environment-based inspection mode is enabled.
pub(crate) fn hide_meshes_on_startup(
    flag: Res<HideMeshesFlag>,
    mut q: Query<&mut Visibility, With<bevy::mesh::Mesh3d>>,
    mut done: Local<bool>,
) {
    if !flag.0 || *done {
        return;
    }
    let mut count = 0;
    for mut v in q.iter_mut() {
        *v = Visibility::Hidden;
        count += 1;
    }
    if count > 0 {
        info!("hide_meshes: hid {count} mesh entities (BEVY_OPENUSD_HIDE_MESHES=1)");
        *done = true;
    }
}

#[derive(Resource)]
pub(crate) struct HideMeshesFlag(pub(crate) bool);

#[derive(Resource)]
pub(crate) struct ShowJointGizmosFlag(pub(crate) bool);
