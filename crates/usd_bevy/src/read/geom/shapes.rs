use openusd::sdf::Path;
use openusd::usd::Stage;

use super::util::{read_double, read_token};

// ── Shapes ──────────────────────────────────────────────────────────────

pub fn read_cube_size(stage: &Stage, prim: &Path) -> anyhow::Result<Option<f64>> {
    read_double(stage, prim, "size")
}

pub fn read_sphere_radius(stage: &Stage, prim: &Path) -> anyhow::Result<Option<f64>> {
    read_double(stage, prim, "radius")
}

#[derive(Debug, Clone, Copy)]
pub struct ReadCylinder {
    pub radius: f64,
    pub height: f64,
    pub axis: Axis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "X" => Self::X,
            "Y" => Self::Y,
            "Z" => Self::Z,
            _ => return None,
        })
    }
}

pub fn read_cylinder(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadCylinder>> {
    let Some(radius) = read_double(stage, prim, "radius")? else {
        return Ok(None);
    };
    let Some(height) = read_double(stage, prim, "height")? else {
        return Ok(None);
    };
    let axis = read_token(stage, prim, "axis")?
        .as_deref()
        .and_then(Axis::parse)
        .unwrap_or(Axis::Z);
    Ok(Some(ReadCylinder {
        radius,
        height,
        axis,
    }))
}

pub fn read_capsule(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadCylinder>> {
    read_cylinder(stage, prim)
}

pub fn read_double_attr(stage: &Stage, prim: &Path, name: &str) -> Option<f64> {
    read_double(stage, prim, name).ok().flatten()
}
