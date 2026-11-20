use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;

use super::types::*;

pub(crate) fn level_fade(cam_dist: f32, step: f32, close_falloff: f32) -> f32 {
    let log_r = (cam_dist / step).max(1e-3).log10();
    let z = (log_r - GAUSS_PEAK) / GAUSS_WIDTH;
    let z_eff = if z < 0.0 { z * close_falloff } else { z };
    (-0.5 * z_eff * z_eff).exp()
}

pub(crate) fn build_level_mesh(cfg: &GroundGrid, step: f32, half: f32, _is_top: bool) -> Mesh {
    let s = cfg.color.to_srgba();
    let base_rgba = [s.red, s.green, s.blue, s.alpha];

    let n = (half / step) as i32;
    let segments = 2 * n;
    let total_segments = (2 * n + 1) * 2 * segments;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity((total_segments * 2) as usize);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity((total_segments * 2) as usize);

    let radial_fade = |x: f32, z: f32| -> f32 {
        let r = (x * x + z * z).sqrt();
        let t = (r / half).clamp(0.0, 1.0);
        let u = ((1.0 - t) / EDGE_FADE_FRAC).clamp(0.0, 1.0);
        u * u * (3.0 - 2.0 * u)
    };

    let vertex_color = |x: f32, z: f32, axis_idx: i32| -> [f32; 4] {
        let major = axis_idx.rem_euclid(MAJOR_EVERY) == 0;
        let boost = if major { MAJOR_BOOST } else { 1.0 };
        [
            base_rgba[0],
            base_rgba[1],
            base_rgba[2],
            (base_rgba[3] * radial_fade(x, z) * boost).clamp(0.0, 1.0),
        ]
    };

    for i in -n..=n {
        let z = i as f32 * step;
        for s in 0..segments {
            let x0 = -half + s as f32 * step;
            let x1 = -half + (s + 1) as f32 * step;
            positions.push([x0, 0.0, z]);
            positions.push([x1, 0.0, z]);
            colors.push(vertex_color(x0, z, i));
            colors.push(vertex_color(x1, z, i));
        }
    }
    for i in -n..=n {
        let x = i as f32 * step;
        for s in 0..segments {
            let z0 = -half + s as f32 * step;
            let z1 = -half + (s + 1) as f32 * step;
            positions.push([x, 0.0, z0]);
            positions.push([x, 0.0, z1]);
            colors.push(vertex_color(x, z0, i));
            colors.push(vertex_color(x, z1, i));
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::LineList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh
}

pub(crate) fn build_dots_mesh(cfg: &GroundGrid, step: f32, half: f32, _is_top: bool) -> Mesh {
    let s = cfg.color.to_srgba();
    let base_rgba = [s.red, s.green, s.blue, s.alpha];

    let n = (half / step) as i32;
    let radius = step * DOT_RADIUS_FRAC;
    let segs = DOT_SEGMENTS;

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for i in -n..=n {
        for j in -n..=n {
            let cx = i as f32 * step;
            let cz = j as f32 * step;
            let r = (cx * cx + cz * cz).sqrt();
            let t = (r / half).clamp(0.0, 1.0);
            let edge_fade = {
                let u = ((1.0 - t) / EDGE_FADE_FRAC).clamp(0.0, 1.0);
                u * u * (3.0 - 2.0 * u)
            };
            let alpha = (base_rgba[3] * edge_fade).clamp(0.0, 1.0);
            let color = [base_rgba[0], base_rgba[1], base_rgba[2], alpha];
            let dot_radius = radius * edge_fade * edge_fade;

            let centre_idx = positions.len() as u32;
            positions.push([cx, 0.0, cz]);
            colors.push(color);
            for k in 0..segs {
                let theta = (k as f32 / segs as f32) * std::f32::consts::TAU;
                let (sn, cs) = theta.sin_cos();
                positions.push([cx + cs * dot_radius, 0.0, cz + sn * dot_radius]);
                colors.push(color);
            }
            for k in 0..segs {
                let next = (k + 1) % segs;
                indices.push(centre_idx);
                indices.push(centre_idx + 1 + k);
                indices.push(centre_idx + 1 + next);
            }
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(bevy::mesh::Indices::U32(indices));
    mesh
}
