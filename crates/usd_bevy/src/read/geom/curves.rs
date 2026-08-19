use openusd::sdf::Path;
use openusd::usd::Stage;

use super::util::{
    read_double_array, read_float_array, read_int_array, read_int_scalar, read_token,
    read_vec2d_array, read_vec2d_scalar, read_vec3f_array,
};

// ── BasisCurves ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveType {
    Linear,
    Cubic,
}

impl CurveType {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "linear" => Some(Self::Linear),
            "cubic" => Some(Self::Cubic),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveBasis {
    Bezier,
    Bspline,
    CatmullRom,
    Hermite,
}

impl CurveBasis {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "bezier" => Some(Self::Bezier),
            "bspline" => Some(Self::Bspline),
            "catmullRom" => Some(Self::CatmullRom),
            "hermite" => Some(Self::Hermite),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveWrap {
    Nonperiodic,
    Periodic,
    Pinned,
}

impl CurveWrap {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "nonperiodic" => Some(Self::Nonperiodic),
            "periodic" => Some(Self::Periodic),
            "pinned" => Some(Self::Pinned),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReadCurves {
    pub points: Vec<[f32; 3]>,
    pub vertex_counts: Vec<i32>,
    pub curve_type: CurveType,
    pub basis: CurveBasis,
    pub wrap: CurveWrap,
    pub widths: Vec<f32>,
    pub display_color: Option<Vec<[f32; 3]>>,
}

pub fn read_curves(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadCurves>> {
    let Some(points) = read_vec3f_array(stage, prim, "points")? else {
        return Ok(None);
    };
    let Some(vertex_counts) = read_int_array(stage, prim, "curveVertexCounts")? else {
        return Ok(None);
    };
    let curve_type = read_token(stage, prim, "type")?
        .as_deref()
        .and_then(CurveType::parse)
        .unwrap_or(CurveType::Linear);
    let basis = read_token(stage, prim, "basis")?
        .as_deref()
        .and_then(CurveBasis::parse)
        .unwrap_or(CurveBasis::Bezier);
    let wrap = read_token(stage, prim, "wrap")?
        .as_deref()
        .and_then(CurveWrap::parse)
        .unwrap_or(CurveWrap::Nonperiodic);
    Ok(Some(ReadCurves {
        points,
        vertex_counts,
        curve_type,
        basis,
        wrap,
        widths: read_float_array(stage, prim, "widths")?.unwrap_or_default(),
        display_color: read_vec3f_array(stage, prim, "primvars:displayColor")?,
    }))
}

// ── NurbsCurves ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ReadNurbsCurves {
    pub points: Vec<[f32; 3]>,
    pub curve_vertex_counts: Vec<i32>,
    pub order: Vec<i32>,
    pub knots: Vec<f64>,
    pub ranges: Vec<[f64; 2]>,
    pub widths: Vec<f32>,
    pub display_color: Option<Vec<[f32; 3]>>,
}

pub fn read_nurbs_curves(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadNurbsCurves>> {
    let Some(points) = read_vec3f_array(stage, prim, "points")? else {
        return Ok(None);
    };
    let Some(curve_vertex_counts) = read_int_array(stage, prim, "curveVertexCounts")? else {
        return Ok(None);
    };
    let order = read_int_array(stage, prim, "order")?
        .unwrap_or_else(|| curve_vertex_counts.iter().map(|_| 4).collect());
    let knots = read_double_array(stage, prim, "knots")?.unwrap_or_default();
    let ranges = read_vec2d_array(stage, prim, "ranges")?.unwrap_or_else(|| {
        let mut out = Vec::with_capacity(curve_vertex_counts.len());
        let mut k_cursor = 0usize;
        for (i, count) in curve_vertex_counts.iter().enumerate() {
            let n = (*count).max(0) as usize;
            let p = order.get(i).copied().unwrap_or(4) as usize;
            let nk = n + p;
            if k_cursor + nk <= knots.len() && p > 0 && n > 0 {
                out.push([knots[k_cursor + p - 1], knots[k_cursor + n]]);
            } else {
                out.push([0.0, 1.0]);
            }
            k_cursor += nk;
        }
        out
    });
    Ok(Some(ReadNurbsCurves {
        points,
        curve_vertex_counts,
        order,
        knots,
        ranges,
        widths: read_float_array(stage, prim, "widths")?.unwrap_or_default(),
        display_color: read_vec3f_array(stage, prim, "primvars:displayColor")?,
    }))
}

// ── NurbsPatch ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ReadNurbsPatch {
    pub points: Vec<[f32; 3]>,
    pub u_vertex_count: i32,
    pub v_vertex_count: i32,
    pub u_order: i32,
    pub v_order: i32,
    pub u_knots: Vec<f64>,
    pub v_knots: Vec<f64>,
    pub u_range: [f64; 2],
    pub v_range: [f64; 2],
    pub display_color: Option<Vec<[f32; 3]>>,
}

pub fn read_nurbs_patch(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadNurbsPatch>> {
    let Some(points) = read_vec3f_array(stage, prim, "points")? else {
        return Ok(None);
    };
    let u_vertex_count = read_int_scalar(stage, prim, "uVertexCount")?.unwrap_or(0);
    let v_vertex_count = read_int_scalar(stage, prim, "vVertexCount")?.unwrap_or(0);
    let u_order = read_int_scalar(stage, prim, "uOrder")?.unwrap_or(4);
    let v_order = read_int_scalar(stage, prim, "vOrder")?.unwrap_or(4);
    let u_knots = read_double_array(stage, prim, "uKnots")?.unwrap_or_default();
    let v_knots = read_double_array(stage, prim, "vKnots")?.unwrap_or_default();
    let u_range = read_vec2d_scalar(stage, prim, "uRange")?
        .unwrap_or_else(|| inner_span(&u_knots, u_vertex_count, u_order));
    let v_range = read_vec2d_scalar(stage, prim, "vRange")?
        .unwrap_or_else(|| inner_span(&v_knots, v_vertex_count, v_order));
    Ok(Some(ReadNurbsPatch {
        points,
        u_vertex_count,
        v_vertex_count,
        u_order,
        v_order,
        u_knots,
        v_knots,
        u_range,
        v_range,
        display_color: read_vec3f_array(stage, prim, "primvars:displayColor")?,
    }))
}

fn inner_span(knots: &[f64], vertex_count: i32, order: i32) -> [f64; 2] {
    let n = vertex_count as usize;
    let p = order as usize;
    if !knots.is_empty() && n > 0 && p > 0 && knots.len() >= n + p {
        [knots[p - 1], knots[n]]
    } else {
        [0.0, 1.0]
    }
}

// ── HermiteCurves ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ReadHermiteCurves {
    pub points: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 3]>,
    pub curve_vertex_counts: Vec<i32>,
    pub widths: Vec<f32>,
    pub display_color: Option<Vec<[f32; 3]>>,
}

pub fn read_hermite_curves(
    stage: &Stage,
    prim: &Path,
) -> anyhow::Result<Option<ReadHermiteCurves>> {
    let Some(points) = read_vec3f_array(stage, prim, "points")? else {
        return Ok(None);
    };
    let Some(curve_vertex_counts) = read_int_array(stage, prim, "curveVertexCounts")? else {
        return Ok(None);
    };
    let tangents = read_vec3f_array(stage, prim, "tangents")?.unwrap_or_else(|| {
        let n = points.len();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let a = points[i];
            let b = if i + 1 < n { points[i + 1] } else { points[i] };
            out.push([b[0] - a[0], b[1] - a[1], b[2] - a[2]]);
        }
        out
    });
    Ok(Some(ReadHermiteCurves {
        points,
        tangents,
        curve_vertex_counts,
        widths: read_float_array(stage, prim, "widths")?.unwrap_or_default(),
        display_color: read_vec3f_array(stage, prim, "primvars:displayColor")?,
    }))
}
