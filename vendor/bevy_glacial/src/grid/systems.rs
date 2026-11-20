use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use super::mesh::{build_dots_mesh, build_level_mesh, level_fade};
use super::types::*;
use crate::camera::ChaseCamera;

pub fn spawn_circle_meshes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    cfg: &GroundGrid,
) {
    let make_mat = |materials: &mut Assets<StandardMaterial>| {
        materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        })
    };

    for level in 0..LEVEL_STEPS.len() {
        let step = LEVEL_STEPS[level];
        let half = LEVEL_HALF[level];
        let is_top = level + 1 == LEVEL_STEPS.len();

        let lines_mesh = meshes.add(build_level_mesh(cfg, step, half, is_top));
        let lines_mat = make_mat(materials);
        commands.spawn((
            Name::new(format!("LocalGrid[L{level}]:Lines")),
            LocalGrid {
                level: level as u8,
                kind: GridKind::Lines,
                material: lines_mat.clone(),
            },
            Transform::default(),
            Mesh3d(lines_mesh),
            MeshMaterial3d(lines_mat),
            NotShadowCaster,
            Visibility::Visible,
        ));

        let dots_mesh = meshes.add(build_dots_mesh(cfg, step, half, is_top));
        let dots_mat = make_mat(materials);
        commands.spawn((
            Name::new(format!("LocalGrid[L{level}]:Dots")),
            LocalGrid {
                level: level as u8,
                kind: GridKind::Dots,
                material: dots_mat.clone(),
            },
            Transform::default(),
            Mesh3d(dots_mesh),
            MeshMaterial3d(dots_mat),
            NotShadowCaster,
            Visibility::Visible,
        ));
    }
}

pub(crate) fn setup_ground_grid(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cfg: Res<GroundGrid>,
    mut mesh_style: ResMut<GroundGridMeshStyle>,
) {
    spawn_circle_meshes(&mut commands, &mut meshes, &mut materials, &cfg);
    mesh_style.color = Some(cfg.color);
}

pub fn build_grid_meshes(
    cameras: Query<&ChaseCamera>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cfg: Res<GroundGrid>,
    mut grids: Query<(&LocalGrid, &mut Transform, &mut Visibility)>,
) {
    let Ok(cam) = cameras.single() else { return };
    let cam_dist = cam.distance.max(0.1);

    for (grid, mut tr, mut vis) in grids.iter_mut() {
        let Some(ground_y) = cfg.ground_y else {
            *vis = Visibility::Hidden;
            continue;
        };

        let level = grid.level as usize;
        let is_coarsest = level + 1 == LEVEL_STEPS.len();
        let level_scale = if is_coarsest {
            (cfg.coverage_radius.max(LEVEL_HALF[level]) / LEVEL_HALF[level]).max(1.0)
        } else {
            1.0
        };
        let step = LEVEL_STEPS[level] * level_scale;
        let snap_step = step * MAJOR_EVERY as f32;
        tr.translation.x = (cam.focus.x / snap_step).round() * snap_step;
        tr.translation.y = match grid.kind {
            GridKind::Lines => ground_y,
            GridKind::Dots => ground_y + step * DOT_OFFSET_FRAC,
        };
        tr.translation.z = (cam.focus.z / snap_step).round() * snap_step;
        tr.scale = Vec3::new(level_scale, 1.0, level_scale);

        let fade = match grid.kind {
            GridKind::Lines => level_fade(cam_dist, step, LINE_CLOSE_FALLOFF),
            GridKind::Dots => level_fade(cam_dist, step, DOT_CLOSE_FALLOFF),
        };
        let a = cfg.color.alpha() * fade;
        *vis = if cfg.visible && a > 0.005 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if let Some(mut m) = materials.get_mut(&grid.material) {
            let srgba = cfg.color.to_srgba();
            m.base_color = Color::srgba(srgba.red, srgba.green, srgba.blue, a);
        }
    }
}

pub fn update_grid_alpha(
    cfg: Res<GroundGrid>,
    mut meshes: ResMut<Assets<Mesh>>,
    grids: Query<(&LocalGrid, &Mesh3d)>,
    mut counters: Option<ResMut<GlacialGridCounters>>,
    mut mesh_style: ResMut<GroundGridMeshStyle>,
) {
    if mesh_style.color == Some(cfg.color) {
        return;
    }
    if let Some(ref mut c) = counters {
        c.alpha_rebuild_calls += 1;
    }
    let mut all_meshes_updated = true;
    for (grid, mesh_h) in grids.iter() {
        let step = LEVEL_STEPS[grid.level as usize];
        let half = LEVEL_HALF[grid.level as usize];
        let is_top = grid.level as usize + 1 == LEVEL_STEPS.len();
        let new_mesh = match grid.kind {
            GridKind::Lines => {
                if let Some(ref mut c) = counters {
                    c.lines_rebuilt += 1;
                }
                build_level_mesh(&cfg, step, half, is_top)
            }
            GridKind::Dots => {
                if let Some(ref mut c) = counters {
                    c.dots_rebuilt += 1;
                }
                build_dots_mesh(&cfg, step, half, is_top)
            }
        };
        if let Some(ref mut c) = counters {
            c.vertices_generated += new_mesh.count_vertices() as u64;
            if let Some(indices) = new_mesh.indices() {
                c.indices_generated += indices.len() as u64;
            }
        }
        if let Some(mut m) = meshes.get_mut(&mesh_h.0) {
            *m = new_mesh;
        } else {
            all_meshes_updated = false;
        }
    }
    if all_meshes_updated {
        mesh_style.color = Some(cfg.color);
    }
}
